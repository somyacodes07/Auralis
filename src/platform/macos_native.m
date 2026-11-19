#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <QuartzCore/QuartzCore.h>

#include <stdint.h>
#include <stdio.h>
#include <string.h>

@interface AuralisWallpaperHost : NSObject
{
@public
    char lastError[512];
}
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) AVPlayer *player;
@property(nonatomic, strong) AVPlayerLayer *playerLayer;
@property(nonatomic, strong) AVPlayer *pendingPlayer;
@property(nonatomic, strong) AVPlayerLayer *pendingLayer;
@property(nonatomic, assign) CGDirectDisplayID displayID;
@property(nonatomic, assign) float playbackRate;
@property(nonatomic, assign) BOOL loopSeekPending;
@end

@implementation AuralisWallpaperHost
@end

@interface AuralisMenuController : NSObject
@property(nonatomic, strong) NSStatusItem *statusItem;
@property(nonatomic, strong) NSMenuItem *launchAtLoginItem;
@property(nonatomic, strong) NSMutableArray *notificationTokens;
- (void)chooseWallpaper:(id)sender;
- (void)togglePause:(id)sender;
- (void)toggleMute:(id)sender;
- (void)nextRecent:(id)sender;
- (void)stopWallpaper:(id)sender;
- (void)toggleLaunchAtLogin:(id)sender;
- (void)quitAuralis:(id)sender;
@end

static NSMutableArray<NSDictionary<NSString *, id> *> *gAuralisActions;
static AuralisMenuController *gAuralisMenu;

static AuralisWallpaperHost *AuralisHost(void *handle) {
    return handle == NULL ? nil : (__bridge AuralisWallpaperHost *)handle;
}

static NSScreen *AuralisScreen(CGDirectDisplayID displayID) {
    for (NSScreen *screen in [NSScreen screens]) {
        NSNumber *number = screen.deviceDescription[@"NSScreenNumber"];
        if (number != nil && number.unsignedIntValue == displayID) {
            return screen;
        }
    }
    return nil;
}

static void AuralisQueueAction(int32_t action, NSString *path) {
    @synchronized ([AuralisMenuController class]) {
        if (gAuralisActions == nil) {
            gAuralisActions = [NSMutableArray array];
        }
        [gAuralisActions addObject:@{@"action" : @(action), @"path" : path ?: @""}];
    }
}

// Auralis mark on its 64-unit grid (y down). These mirror A_STROKE, WAVE_STROKE and
// STROKE_WIDTH in src/brand.rs, the single source of truth; the Rust test
// brand::tests::macos_template_image_matches_brand_geometry fails if they drift.
static const CGFloat kAuralisMarkA[][2] = {
    {13.0, 51.0}, {32.0, 13.0}, {51.0, 51.0},
};
static const CGFloat kAuralisMarkWave[][2] = {
    {19.0, 39.0}, {20.3, 38.75}, {21.6, 38.06}, {22.9, 37.09}, {24.2, 36.1}, {25.5, 35.33},
    {26.8, 35.0}, {28.1, 35.25}, {29.4, 36.1}, {30.7, 37.41}, {32.0, 39.0}, {33.3, 40.59},
    {34.6, 41.9}, {35.9, 42.75}, {37.2, 43.0}, {38.5, 42.67}, {39.8, 41.9}, {41.1, 40.91},
    {42.4, 39.94}, {43.7, 39.25}, {45.0, 39.0},
};
static const CGFloat kAuralisMarkStrokeWidth = 4.0;

static void AuralisAppendPolyline(NSBezierPath *path, const CGFloat (*points)[2], size_t count,
                                  CGFloat scale, CGFloat side) {
    for (size_t i = 0; i < count; i++) {
        // Center the grid in the image and flip y for AppKit's bottom-left origin.
        NSPoint point = NSMakePoint(side / 2.0 + (points[i][0] - 32.0) * scale,
                                    side / 2.0 - (points[i][1] - 32.0) * scale);
        if (i == 0) {
            [path moveToPoint:point];
        } else {
            [path lineToPoint:point];
        }
    }
}

static NSImage *AuralisMarkImage(void) {
    // The inked mark spans 42 grid units (x 11..53 with stroke); fit it to 16 of the 18 pt
    // menu-bar image, which gives a 1.5 pt stroke.
    const CGFloat side = 18.0;
    const CGFloat scale = 16.0 / 42.0;
    NSImage *image = [[NSImage alloc] initWithSize:NSMakeSize(side, side)];
    [image lockFocus];
    [[NSColor whiteColor] setStroke];

    NSBezierPath *mark = [NSBezierPath bezierPath];
    mark.lineWidth = kAuralisMarkStrokeWidth * scale;
    mark.lineCapStyle = NSLineCapStyleRound;
    mark.lineJoinStyle = NSLineJoinStyleRound;
    AuralisAppendPolyline(mark, kAuralisMarkA, sizeof(kAuralisMarkA) / sizeof(kAuralisMarkA[0]),
                          scale, side);
    AuralisAppendPolyline(mark, kAuralisMarkWave,
                          sizeof(kAuralisMarkWave) / sizeof(kAuralisMarkWave[0]), scale, side);
    [mark stroke];
    [image unlockFocus];
    image.template = YES;
    return image;
}

static NSMenuItem *AuralisMenuItem(NSString *title, SEL action, id target) {
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:title action:action keyEquivalent:@""];
    item.target = target;
    return item;
}

@implementation AuralisMenuController
- (void)chooseWallpaper:(id)sender {
    (void)sender;
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = YES;
    panel.canChooseDirectories = NO;
    panel.allowsMultipleSelection = NO;
    panel.allowedFileTypes = @[@"mp4", @"mov", @"mkv", @"webm", @"m4v"];
    panel.prompt = @"Set Wallpaper";
    if ([panel runModal] == NSModalResponseOK && panel.URL != nil) {
        AuralisQueueAction(1, panel.URL.path);
    }
}

- (void)togglePause:(id)sender {
    (void)sender;
    AuralisQueueAction(2, nil);
}

- (void)toggleMute:(id)sender {
    (void)sender;
    AuralisQueueAction(3, nil);
}

- (void)nextRecent:(id)sender {
    (void)sender;
    AuralisQueueAction(4, nil);
}

- (void)stopWallpaper:(id)sender {
    (void)sender;
    AuralisQueueAction(5, nil);
}

- (void)toggleLaunchAtLogin:(id)sender {
    (void)sender;
    BOOL enable = self.launchAtLoginItem.state != NSControlStateValueOn;
    self.launchAtLoginItem.state = enable ? NSControlStateValueOn : NSControlStateValueOff;
    AuralisQueueAction(10, enable ? @"1" : @"0");
}

- (void)quitAuralis:(id)sender {
    (void)sender;
    AuralisQueueAction(6, nil);
}
@end

void auralis_macos_begin_menu(uint8_t launchAtLogin) {
    @autoreleasepool {
        if (gAuralisMenu != nil) {
            return;
        }

        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyAccessory];
        @synchronized ([AuralisMenuController class]) {
            gAuralisActions = [NSMutableArray array];
        }

        AuralisMenuController *controller = [[AuralisMenuController alloc] init];
        controller.statusItem = [[NSStatusBar systemStatusBar] statusItemWithLength:NSVariableStatusItemLength];
        controller.statusItem.button.image = AuralisMarkImage();
        controller.statusItem.button.toolTip = @"Auralis";

        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Auralis"];
        [menu addItem:AuralisMenuItem(@"Set Primary Wallpaper…", @selector(chooseWallpaper:), controller)];
        [menu addItem:AuralisMenuItem(@"Next Recent Wallpaper", @selector(nextRecent:), controller)];
        [menu addItem:[NSMenuItem separatorItem]];
        [menu addItem:AuralisMenuItem(@"Pause / Resume", @selector(togglePause:), controller)];
        [menu addItem:AuralisMenuItem(@"Mute / Unmute", @selector(toggleMute:), controller)];
        [menu addItem:AuralisMenuItem(@"Stop Wallpaper", @selector(stopWallpaper:), controller)];
        [menu addItem:[NSMenuItem separatorItem]];
        controller.launchAtLoginItem = AuralisMenuItem(@"Launch at Login", @selector(toggleLaunchAtLogin:), controller);
        controller.launchAtLoginItem.state = launchAtLogin ? NSControlStateValueOn : NSControlStateValueOff;
        [menu addItem:controller.launchAtLoginItem];
        [menu addItem:[NSMenuItem separatorItem]];
        [menu addItem:AuralisMenuItem(@"Quit Auralis", @selector(quitAuralis:), controller)];
        controller.statusItem.menu = menu;
        gAuralisMenu = controller;

        controller.notificationTokens = [NSMutableArray array];
        id displayToken = [[NSNotificationCenter defaultCenter]
            addObserverForName:NSApplicationDidChangeScreenParametersNotification
                        object:application
                         queue:nil
                    usingBlock:^(NSNotification *notification) {
                        (void)notification;
                        AuralisQueueAction(7, nil);
                    }];
        [controller.notificationTokens addObject:displayToken];

        NSNotificationCenter *workspaceCenter = [[NSWorkspace sharedWorkspace] notificationCenter];
        id sleepToken = [workspaceCenter addObserverForName:NSWorkspaceWillSleepNotification
                                                    object:nil
                                                     queue:nil
                                                usingBlock:^(NSNotification *notification) {
                                                    (void)notification;
                                                    AuralisQueueAction(8, nil);
                                                }];
        id wakeToken = [workspaceCenter addObserverForName:NSWorkspaceDidWakeNotification
                                                   object:nil
                                                    queue:nil
                                               usingBlock:^(NSNotification *notification) {
                                                   (void)notification;
                                                   AuralisQueueAction(9, nil);
                                               }];
        [controller.notificationTokens addObject:sleepToken];
        [controller.notificationTokens addObject:wakeToken];
    }
}

void auralis_macos_set_launch_menu_state(uint8_t enabled) {
    @autoreleasepool {
        if (gAuralisMenu.launchAtLoginItem != nil) {
            gAuralisMenu.launchAtLoginItem.state = enabled ? NSControlStateValueOn : NSControlStateValueOff;
        }
    }
}

void auralis_macos_end_menu(void) {
    @autoreleasepool {
        if (gAuralisMenu == nil) {
            return;
        }
        for (id token in gAuralisMenu.notificationTokens) {
            [[NSNotificationCenter defaultCenter] removeObserver:token];
            [[[NSWorkspace sharedWorkspace] notificationCenter] removeObserver:token];
        }
        [[NSStatusBar systemStatusBar] removeStatusItem:gAuralisMenu.statusItem];
        gAuralisMenu = nil;
        @synchronized ([AuralisMenuController class]) {
            gAuralisActions = nil;
        }
    }
}

void auralis_macos_pump_events(void) {
    @autoreleasepool {
        NSApplication *application = [NSApplication sharedApplication];
        for (NSUInteger index = 0; index < 64; index++) {
            NSEvent *event = [application nextEventMatchingMask:NSEventMaskAny
                                                      untilDate:[NSDate dateWithTimeIntervalSinceNow:0.0]
                                                         inMode:NSDefaultRunLoopMode
                                                        dequeue:YES];
            if (event == nil) {
                break;
            }
            [application sendEvent:event];
        }
        [application updateWindows];
    }
}

int32_t auralis_macos_poll_action(char *path, size_t pathCapacity) {
    @autoreleasepool {
        NSDictionary<NSString *, id> *event = nil;
        @synchronized ([AuralisMenuController class]) {
            if (gAuralisActions.count == 0) {
                return 0;
            }
            event = gAuralisActions.firstObject;
            [gAuralisActions removeObjectAtIndex:0];
        }
        NSString *value = event[@"path"];
        if (path != NULL && pathCapacity > 0) {
            memset(path, 0, pathCapacity);
            if (value.length > 0) {
                (void)[value getCString:path maxLength:pathCapacity encoding:NSUTF8StringEncoding];
            }
        }
        return [event[@"action"] intValue];
    }
}

void auralis_macos_show_message(const char *title, const char *message) {
    @autoreleasepool {
        NSString *titleString = title == NULL ? @"Auralis" : [NSString stringWithUTF8String:title];
        NSString *messageString = message == NULL ? @"" : [NSString stringWithUTF8String:message];
        NSAlert *alert = [[NSAlert alloc] init];
        alert.messageText = titleString ?: @"Auralis";
        alert.informativeText = messageString ?: @"";
        [alert addButtonWithTitle:@"OK"];
        (void)[alert runModal];
    }
}

void *auralis_macos_host_create(uint32_t displayID) {
    @autoreleasepool {
        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyAccessory];
        NSScreen *screen = AuralisScreen(displayID);
        if (screen == nil) {
            return NULL;
        }

        AuralisWallpaperHost *host = [[AuralisWallpaperHost alloc] init];
        host.displayID = displayID;
        host.playbackRate = 1.0f;
        NSWindow *window = [[NSWindow alloc] initWithContentRect:screen.frame
                                                       styleMask:NSWindowStyleMaskBorderless
                                                         backing:NSBackingStoreBuffered
                                                           defer:NO
                                                          screen:screen];
        if (window == nil) {
            return NULL;
        }
        window.releasedWhenClosed = NO;
        window.opaque = YES;
        window.hasShadow = NO;
        window.ignoresMouseEvents = YES;
        window.backgroundColor = [NSColor blackColor];
        window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
                                   NSWindowCollectionBehaviorStationary |
                                   NSWindowCollectionBehaviorIgnoresCycle;

        NSView *content = [[NSView alloc] initWithFrame:window.contentView.bounds];
        content.wantsLayer = YES;
        content.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        content.layer.backgroundColor = [NSColor blackColor].CGColor;
        window.contentView = content;
        host.window = window;
        return (void *)CFBridgingRetain(host);
    }
}

int32_t auralis_macos_host_attach(void *handle) {
    @autoreleasepool {
        AuralisWallpaperHost *host = AuralisHost(handle);
        if (host.window == nil) {
            return 0;
        }
        // This is the standard desktop window level; desktop icons and ordinary windows remain above it.
        host.window.level = CGWindowLevelForKey(kCGDesktopWindowLevelKey);
        [host.window orderFrontRegardless];
        return 1;
    }
}

int32_t auralis_macos_host_resize(void *handle) {
    @autoreleasepool {
        AuralisWallpaperHost *host = AuralisHost(handle);
        NSScreen *screen = AuralisScreen(host.displayID);
        if (host.window == nil || screen == nil) {
            return 0;
        }
        [host.window setFrame:screen.frame display:YES];
        host.playerLayer.frame = host.window.contentView.bounds;
        return 1;
    }
}

int32_t auralis_macos_host_start(void *handle,
                                const char *path,
                                uint8_t muted,
                                uint8_t volume,
                                uint16_t speedPercent,
                                uint8_t fitAspect) {
    @autoreleasepool {
        AuralisWallpaperHost *host = AuralisHost(handle);
        if (host.window == nil || path == NULL) {
            return 0;
        }
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path
                                                   isDirectory:NO
                                                 relativeToURL:nil];
        if (url == nil) {
            snprintf(host->lastError, sizeof(host->lastError), "%s", "Invalid video file path");
            return 0;
        }
        AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];
        AVPlayer *player = [AVPlayer playerWithPlayerItem:item];
        player.actionAtItemEnd = AVPlayerActionAtItemEndNone;
        player.automaticallyWaitsToMinimizeStalling = YES;
        player.allowsExternalPlayback = NO;
        player.muted = muted != 0;
        player.volume = (float)volume / 100.0f;
        host.playbackRate = (float)speedPercent / 100.0f;
        player.defaultRate = host.playbackRate;

        AVPlayerLayer *layer = [AVPlayerLayer playerLayerWithPlayer:player];
        layer.frame = host.window.contentView.bounds;
        layer.autoresizingMask = kCALayerWidthSizable | kCALayerHeightSizable;
        layer.opacity = 0.0;
        layer.videoGravity = fitAspect ? AVLayerVideoGravityResizeAspect : AVLayerVideoGravityResizeAspectFill;
        [host.window.contentView.layer addSublayer:layer];
        [host.pendingPlayer pause];
        [host.pendingLayer removeFromSuperlayer];
        host.pendingPlayer = player;
        host.pendingLayer = layer;
        host.loopSeekPending = NO;
        memset(host->lastError, 0, sizeof(host->lastError));
        [player playImmediatelyAtRate:host.playbackRate];
        return 1;
    }
}

int32_t auralis_macos_host_pause(void *handle) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    AVPlayer *player = host.pendingPlayer ?: host.player;
    if (player == nil) {
        return 0;
    }
    [player pause];
    return 1;
}

int32_t auralis_macos_host_resume(void *handle) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    AVPlayer *player = host.pendingPlayer ?: host.player;
    if (player == nil) {
        return 0;
    }
    [player playImmediatelyAtRate:host.playbackRate];
    return 1;
}

int32_t auralis_macos_host_stop(void *handle) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    if (host.player == nil) {
        if (host.pendingPlayer == nil) {
            return 1;
        }
    }
    [host.player pause];
    [host.player replaceCurrentItemWithPlayerItem:nil];
    [host.playerLayer removeFromSuperlayer];
    [host.pendingPlayer pause];
    [host.pendingPlayer replaceCurrentItemWithPlayerItem:nil];
    [host.pendingLayer removeFromSuperlayer];
    host.playerLayer = nil;
    host.player = nil;
    host.pendingLayer = nil;
    host.pendingPlayer = nil;
    host.loopSeekPending = NO;
    return 1;
}

int32_t auralis_macos_host_set_audio(void *handle, uint8_t muted, uint8_t volume) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    AVPlayer *player = host.pendingPlayer ?: host.player;
    if (player == nil) {
        return 0;
    }
    player.muted = muted != 0;
    player.volume = (float)volume / 100.0f;
    return 1;
}

int32_t auralis_macos_host_set_rate(void *handle, uint16_t speedPercent) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    AVPlayer *player = host.pendingPlayer ?: host.player;
    if (player == nil) {
        return 0;
    }
    host.playbackRate = (float)speedPercent / 100.0f;
    player.defaultRate = host.playbackRate;
    if (player.rate > 0.0f) {
        [player playImmediatelyAtRate:host.playbackRate];
    }
    return 1;
}

int32_t auralis_macos_host_set_aspect(void *handle, uint8_t fitAspect) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    AVPlayerLayer *layer = host.pendingLayer ?: host.playerLayer;
    if (layer == nil) {
        return 0;
    }
    layer.videoGravity = fitAspect ? AVLayerVideoGravityResizeAspect : AVLayerVideoGravityResizeAspectFill;
    return 1;
}

int32_t auralis_macos_host_tick(void *handle, uint8_t loopPlayback) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    AVPlayerItem *pendingItem = host.pendingPlayer.currentItem;
    if (pendingItem.status == AVPlayerItemStatusFailed) {
        const char *description = pendingItem.error.localizedDescription.UTF8String;
        snprintf(host->lastError, sizeof(host->lastError), "%s", description ?: "AVFoundation could not load this video");
        [host.pendingPlayer pause];
        [host.pendingLayer removeFromSuperlayer];
        host.pendingPlayer = nil;
        host.pendingLayer = nil;
        return -1;
    }
    if (host.pendingPlayer != nil && pendingItem.status == AVPlayerItemStatusReadyToPlay) {
        [host.player pause];
        [host.playerLayer removeFromSuperlayer];
        host.player = host.pendingPlayer;
        host.playerLayer = host.pendingLayer;
        host.pendingPlayer = nil;
        host.pendingLayer = nil;
        host.playerLayer.opacity = 1.0;
        host.loopSeekPending = NO;
    }
    AVPlayerItem *item = host.player.currentItem;
    if (item == nil) {
        return 0;
    }
    if (item.status == AVPlayerItemStatusFailed) {
        const char *description = item.error.localizedDescription.UTF8String;
        snprintf(host->lastError, sizeof(host->lastError), "%s", description ?: "AVFoundation playback failed");
        return -1;
    }
    if (loopPlayback == 0 || item.status != AVPlayerItemStatusReadyToPlay || host.loopSeekPending) {
        return 0;
    }

    CMTime duration = item.duration;
    CMTime position = host.player.currentTime;
    if (!CMTIME_IS_NUMERIC(duration) || !CMTIME_IS_NUMERIC(position)) {
        return 0;
    }
    CMTime loopPoint = CMTimeSubtract(duration, CMTimeMakeWithSeconds(0.15, 600));
    if (CMTimeCompare(duration, CMTimeMakeWithSeconds(0.15, 600)) > 0 &&
        CMTimeCompare(position, loopPoint) >= 0) {
        host.loopSeekPending = YES;
        __weak AuralisWallpaperHost *weakHost = host;
        AVPlayer *player = host.player;
        [player seekToTime:kCMTimeZero
                toleranceBefore:kCMTimeZero
                 toleranceAfter:kCMTimeZero
              completionHandler:^(BOOL finished) {
                  AuralisWallpaperHost *strongHost = weakHost;
                  if (strongHost == nil) {
                      return;
                  }
                  strongHost.loopSeekPending = NO;
                  if (finished && strongHost.player == player) {
                      [player playImmediatelyAtRate:strongHost.playbackRate];
                  }
              }];
    }
    return 0;
}

const char *auralis_macos_host_last_error(void *handle) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    return host == nil ? "AVFoundation playback failed" : host->lastError;
}

int32_t auralis_macos_host_detach(void *handle) {
    AuralisWallpaperHost *host = AuralisHost(handle);
    if (host.window == nil) {
        return 0;
    }
    [host.window orderOut:nil];
    return 1;
}

int32_t auralis_macos_host_destroy(void *handle) {
    @autoreleasepool {
        AuralisWallpaperHost *host = AuralisHost(handle);
        if (host == nil) {
            return 1;
        }
        (void)auralis_macos_host_stop(handle);
        [host.window orderOut:nil];
        [host.window close];
        host.window = nil;
        (void)CFBridgingRelease(handle);
        return 1;
    }
}
