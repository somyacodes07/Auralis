#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <IOKit/ps/IOPSKeys.h>
#import <IOKit/ps/IOPowerSources.h>
#import <QuartzCore/QuartzCore.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

#include <mach/mach.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>

// Small AppKit bridge for Auralis. Objective-C here owns views and native players only.
// Every user action is queued as {action code, integer value, UTF-8 text} and drained by Rust
// (`auralis_macos_poll_action`); Rust decides what to do and pushes display state back through
// the `auralis_macos_studio_*` functions. Action codes must match `src/platform/macos_studio.rs`.
enum {
    AuralisActionSetPrimaryWallpaper = 1,
    AuralisActionTogglePauseAll = 2,
    AuralisActionToggleMuteAll = 3,
    AuralisActionNextRecentPrimary = 4,
    AuralisActionStopAll = 5,
    AuralisActionQuit = 6,
    AuralisActionDisplaysChanged = 7,
    AuralisActionSystemWillSleep = 8,
    AuralisActionSystemDidWake = 9,
    AuralisActionSetLaunchAtLogin = 10,
    AuralisActionOpenStudio = 11,
    AuralisActionStudioClosed = 12,
    AuralisActionSelectDisplay = 13,
    AuralisActionPreview = 14,
    AuralisActionStopPreview = 15,
    AuralisActionApply = 16,
    AuralisActionTogglePause = 17,
    AuralisActionStop = 18,
    AuralisActionSetMuted = 19,
    AuralisActionSetLoop = 20,
    AuralisActionStepVolume = 21,
    AuralisActionStepSpeed = 22,
    AuralisActionSetAspect = 23,
    AuralisActionSetPauseOnFullscreen = 24,
    AuralisActionSetPauseOnBattery = 25,
    AuralisActionSetReduceMotion = 26,
    AuralisActionNextRecent = 27,
};

// Must match `NativeStudioState` in `src/platform/macos.rs`.
typedef struct {
    const char *status;
    const char *details;
    const char *resources;
    const char *volumeLabel;
    const char *speedLabel;
    const char *pauseLabel;
    uint8_t controlsEnabled;
    uint8_t muted;
    uint8_t loopPlayback;
    uint8_t fitAspect;
    uint8_t pauseOnFullscreen;
    uint8_t pauseOnBattery;
    uint8_t launchAtLogin;
    uint8_t reduceMotion;
    uint8_t previewActive;
} AuralisStudioState;

// Must match `NativeRect` in `src/platform/macos.rs`.
typedef struct {
    double x;
    double y;
    double width;
    double height;
} AuralisRect;

static BOOL gAuralisReduceMotion = NO;

static BOOL AuralisShouldReduceMotion(void) {
    return gAuralisReduceMotion || [NSWorkspace sharedWorkspace].accessibilityDisplayShouldReduceMotion;
}

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
- (void)openStudio:(id)sender;
- (void)quitAuralis:(id)sender;
@end

@class AuralisStudioController;

static NSMutableArray<NSDictionary<NSString *, id> *> *gAuralisActions;
static AuralisMenuController *gAuralisMenu;
static AuralisStudioController *gAuralisStudio;
static void AuralisStudioClose(void);

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

static void AuralisQueueAction(int32_t action, int32_t value, NSString *text) {
    @synchronized ([AuralisMenuController class]) {
        if (gAuralisActions == nil) {
            gAuralisActions = [NSMutableArray array];
        }
        [gAuralisActions addObject:@{@"action" : @(action), @"value" : @(value), @"text" : text ?: @""}];
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

static NSString *AuralisString(const char *text) {
    if (text == NULL) {
        return @"";
    }
    return [NSString stringWithUTF8String:text] ?: @"";
}

static NSArray<NSString *> *AuralisVideoExtensions(void) {
    // Must match `SUPPORTED_EXTENSIONS` in `src/config.rs`.
    return @[@"mp4", @"mov", @"mkv", @"webm", @"m4v"];
}

static NSArray<UTType *> *AuralisVideoTypes(void) {
    NSMutableArray<UTType *> *types = [NSMutableArray array];
    for (NSString *extension in AuralisVideoExtensions()) {
        UTType *type = [UTType typeWithFilenameExtension:extension];
        if (type != nil && ![types containsObject:type]) {
            [types addObject:type];
        }
    }
    return types;
}

static BOOL AuralisIsVideoURL(NSURL *url) {
    return url.isFileURL && [AuralisVideoExtensions() containsObject:url.pathExtension.lowercaseString];
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
    panel.allowedContentTypes = AuralisVideoTypes();
    panel.prompt = @"Set Wallpaper";
    if ([panel runModal] == NSModalResponseOK && panel.URL != nil) {
        AuralisQueueAction(AuralisActionSetPrimaryWallpaper, 0, panel.URL.path);
    }
}

- (void)togglePause:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionTogglePauseAll, 0, nil);
}

- (void)toggleMute:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionToggleMuteAll, 0, nil);
}

- (void)nextRecent:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionNextRecentPrimary, 0, nil);
}

- (void)stopWallpaper:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionStopAll, 0, nil);
}

- (void)toggleLaunchAtLogin:(id)sender {
    (void)sender;
    BOOL enable = self.launchAtLoginItem.state != NSControlStateValueOn;
    self.launchAtLoginItem.state = enable ? NSControlStateValueOn : NSControlStateValueOff;
    AuralisQueueAction(AuralisActionSetLaunchAtLogin, enable ? 1 : 0, nil);
}

- (void)openStudio:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionOpenStudio, 0, nil);
}

- (void)quitAuralis:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionQuit, 0, nil);
}
@end

// Main menu: only reachable while the Studio is open (the app is otherwise an accessory), it
// provides the standard key equivalents, including text editing in the path field.
static NSMenu *AuralisMainMenu(AuralisMenuController *controller) {
    NSMenu *mainMenu = [[NSMenu alloc] initWithTitle:@""];

    NSMenuItem *appItem = [[NSMenuItem alloc] initWithTitle:@"Auralis" action:nil keyEquivalent:@""];
    NSMenu *appMenu = [[NSMenu alloc] initWithTitle:@"Auralis"];
    NSMenuItem *studio = [[NSMenuItem alloc] initWithTitle:@"Open Auralis Studio…"
                                                    action:@selector(openStudio:)
                                             keyEquivalent:@","];
    studio.target = controller;
    [appMenu addItem:studio];
    [appMenu addItem:[NSMenuItem separatorItem]];
    [appMenu addItemWithTitle:@"Hide Auralis" action:@selector(hide:) keyEquivalent:@"h"];
    [appMenu addItem:[NSMenuItem separatorItem]];
    NSMenuItem *quit = [[NSMenuItem alloc] initWithTitle:@"Quit Auralis"
                                                  action:@selector(quitAuralis:)
                                           keyEquivalent:@"q"];
    quit.target = controller;
    [appMenu addItem:quit];
    appItem.submenu = appMenu;
    [mainMenu addItem:appItem];

    NSMenuItem *editItem = [[NSMenuItem alloc] initWithTitle:@"Edit" action:nil keyEquivalent:@""];
    NSMenu *editMenu = [[NSMenu alloc] initWithTitle:@"Edit"];
    [editMenu addItemWithTitle:@"Undo" action:@selector(undo:) keyEquivalent:@"z"];
    NSMenuItem *redo = [editMenu addItemWithTitle:@"Redo" action:@selector(redo:) keyEquivalent:@"z"];
    redo.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagShift;
    [editMenu addItem:[NSMenuItem separatorItem]];
    [editMenu addItemWithTitle:@"Cut" action:@selector(cut:) keyEquivalent:@"x"];
    [editMenu addItemWithTitle:@"Copy" action:@selector(copy:) keyEquivalent:@"c"];
    [editMenu addItemWithTitle:@"Paste" action:@selector(paste:) keyEquivalent:@"v"];
    [editMenu addItemWithTitle:@"Select All" action:@selector(selectAll:) keyEquivalent:@"a"];
    editItem.submenu = editMenu;
    [mainMenu addItem:editItem];

    NSMenuItem *windowItem = [[NSMenuItem alloc] initWithTitle:@"Window" action:nil keyEquivalent:@""];
    NSMenu *windowMenu = [[NSMenu alloc] initWithTitle:@"Window"];
    [windowMenu addItemWithTitle:@"Minimize" action:@selector(performMiniaturize:) keyEquivalent:@"m"];
    [windowMenu addItemWithTitle:@"Close" action:@selector(performClose:) keyEquivalent:@"w"];
    windowItem.submenu = windowMenu;
    [mainMenu addItem:windowItem];
    return mainMenu;
}

void auralis_macos_begin_menu(uint8_t launchAtLogin) {
    @autoreleasepool {
        if (gAuralisMenu != nil) {
            return;
        }

        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyAccessory];
        // Rust pumps events manually instead of calling -run, so finish launching explicitly
        // (windows can then become key and the main menu's key equivalents work).
        [application finishLaunching];
        @synchronized ([AuralisMenuController class]) {
            gAuralisActions = [NSMutableArray array];
        }

        AuralisMenuController *controller = [[AuralisMenuController alloc] init];
        controller.statusItem = [[NSStatusBar systemStatusBar] statusItemWithLength:NSVariableStatusItemLength];
        controller.statusItem.button.image = AuralisMarkImage();
        controller.statusItem.button.toolTip = @"Auralis";
        application.mainMenu = AuralisMainMenu(controller);

        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Auralis"];
        NSMenuItem *studioItem = AuralisMenuItem(@"Open Auralis Studio…", @selector(openStudio:), controller);
        studioItem.keyEquivalent = @",";
        studioItem.keyEquivalentModifierMask = NSEventModifierFlagCommand;
        [menu addItem:studioItem];
        [menu addItem:[NSMenuItem separatorItem]];
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
                        AuralisQueueAction(AuralisActionDisplaysChanged, 0, nil);
                    }];
        [controller.notificationTokens addObject:displayToken];

        NSNotificationCenter *workspaceCenter = [[NSWorkspace sharedWorkspace] notificationCenter];
        id sleepToken = [workspaceCenter addObserverForName:NSWorkspaceWillSleepNotification
                                                    object:nil
                                                     queue:nil
                                                usingBlock:^(NSNotification *notification) {
                                                    (void)notification;
                                                    AuralisQueueAction(AuralisActionSystemWillSleep, 0, nil);
                                                }];
        id wakeToken = [workspaceCenter addObserverForName:NSWorkspaceDidWakeNotification
                                                   object:nil
                                                    queue:nil
                                               usingBlock:^(NSNotification *notification) {
                                                   (void)notification;
                                                   AuralisQueueAction(AuralisActionSystemDidWake, 0, nil);
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
        AuralisStudioClose();
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
        // Commit pending Core Animation changes (layer swaps) because this loop is pumped
        // manually instead of by -[NSApplication run].
        [CATransaction flush];
    }
}

int32_t auralis_macos_poll_action(char *text, size_t textCapacity, int32_t *value) {
    @autoreleasepool {
        NSDictionary<NSString *, id> *event = nil;
        @synchronized ([AuralisMenuController class]) {
            if (gAuralisActions.count == 0) {
                return 0;
            }
            event = gAuralisActions.firstObject;
            [gAuralisActions removeObjectAtIndex:0];
        }
        NSString *payload = event[@"text"];
        if (text != NULL && textCapacity > 0) {
            memset(text, 0, textCapacity);
            if (payload.length > 0 &&
                ![payload getCString:text maxLength:textCapacity encoding:NSUTF8StringEncoding]) {
                // Too long for the buffer: deliver an empty payload rather than a truncated path.
                memset(text, 0, textCapacity);
            }
        }
        if (value != NULL) {
            *value = [event[@"value"] intValue];
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
        // With a `screen:` argument AppKit interprets the content rect relative to that screen's
        // origin, so passing the global `screen.frame` would push windows on secondary displays
        // off screen. Create at the local origin, then set the global frame explicitly.
        NSRect localRect = NSMakeRect(0.0, 0.0, screen.frame.size.width, screen.frame.size.height);
        NSWindow *window = [[NSWindow alloc] initWithContentRect:localRect
                                                       styleMask:NSWindowStyleMaskBorderless
                                                         backing:NSBackingStoreBuffered
                                                           defer:NO
                                                          screen:screen];
        if (window == nil) {
            return NULL;
        }
        [window setFrame:screen.frame display:NO];
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
        // kCGDesktopWindowLevel is the level the system desktop picture itself uses, so a window
        // there can stay hidden behind the picture. One level below the Finder desktop-icon
        // level sits above the desktop picture while icons and ordinary windows stay above it.
        host.window.level = CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) - 1;
        NSScreen *screen = AuralisScreen(host.displayID);
        if (screen != nil) {
            [host.window setFrame:screen.frame display:YES];
        }
        [host.window orderFrontRegardless];
        if (!host.window.isVisible) {
            snprintf(host->lastError, sizeof(host->lastError), "%s",
                     "macOS did not show the wallpaper window");
            return 0;
        }
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
        // The new video fades in (Core Animation's implicit 0.25 s opacity animation). Reduce Motion (Auralis setting or the macOS accessibility setting) makes
        // the swap instant.
        [CATransaction begin];
        [CATransaction setDisableActions:AuralisShouldReduceMotion()];
        host.playerLayer.opacity = 1.0;
        [CATransaction commit];
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

int32_t auralis_macos_host_describe(void *handle, char *buffer, size_t capacity) {
    @autoreleasepool {
        AuralisWallpaperHost *host = AuralisHost(handle);
        if (buffer == NULL || capacity == 0) {
            return 0;
        }
        buffer[0] = ' ';
        if (host.window == nil) {
            snprintf(buffer, capacity, "%s", "no host window");
            return 0;
        }
        NSWindow *window = host.window;
        NSRect frame = window.frame;
        NSScreen *screen = AuralisScreen(host.displayID);
        NSRect screenFrame = screen != nil ? screen.frame : NSZeroRect;
        snprintf(buffer, capacity,
                 "window %ld, level %ld (desktop %d, desktop icons %d), frame %.0f,%.0f %.0fx%.0f, "
                 "screen %s %.0f,%.0f %.0fx%.0f, visible %s, on active space %s, occlusion visible %s",
                 (long)window.windowNumber, (long)window.level,
                 (int)CGWindowLevelForKey(kCGDesktopWindowLevelKey),
                 (int)CGWindowLevelForKey(kCGDesktopIconWindowLevelKey), frame.origin.x, frame.origin.y,
                 frame.size.width, frame.size.height, screen != nil ? "found" : "missing",
                 screenFrame.origin.x, screenFrame.origin.y, screenFrame.size.width, screenFrame.size.height,
                 window.isVisible ? "yes" : "no", window.isOnActiveSpace ? "yes" : "no",
                 (window.occlusionState & NSWindowOcclusionStateVisible) ? "yes" : "no");
        return 1;
    }
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

// MARK: - System queries used by the Rust adapter

int32_t auralis_macos_host_telemetry(void *handle,
                                     uint32_t *width,
                                     uint32_t *height,
                                     int64_t *positionMs,
                                     int64_t *durationMs) {
    @autoreleasepool {
        AuralisWallpaperHost *host = AuralisHost(handle);
        if (width != NULL) {
            *width = 0;
        }
        if (height != NULL) {
            *height = 0;
        }
        if (positionMs != NULL) {
            *positionMs = -1;
        }
        if (durationMs != NULL) {
            *durationMs = -1;
        }
        AVPlayer *player = host.player;
        AVPlayerItem *item = player.currentItem;
        if (item == nil) {
            // A replacement still loading counts as active, with no details yet.
            return host.pendingPlayer.currentItem != nil ? 1 : 0;
        }
        CGSize size = item.presentationSize;
        if (size.width > 0.0 && size.height > 0.0) {
            if (width != NULL) {
                *width = (uint32_t)llround(size.width);
            }
            if (height != NULL) {
                *height = (uint32_t)llround(size.height);
            }
        }
        CMTime position = player.currentTime;
        if (CMTIME_IS_NUMERIC(position) && positionMs != NULL) {
            *positionMs = (int64_t)llround(CMTimeGetSeconds(position) * 1000.0);
        }
        CMTime duration = item.duration;
        if (CMTIME_IS_NUMERIC(duration) && durationMs != NULL) {
            *durationMs = (int64_t)llround(CMTimeGetSeconds(duration) * 1000.0);
        }
        return 1;
    }
}

// Writes normal-layer (layer 0), on-screen windows owned by other processes. Bounds are in global
// display coordinates with a top-left origin, matching CGDisplayBounds. Reading bounds, layer
// and owner PID does not need Screen Recording permission; window titles are never read.
size_t auralis_macos_copy_window_rects(AuralisRect *rects, size_t capacity) {
    @autoreleasepool {
        if (rects == NULL || capacity == 0) {
            return 0;
        }
        CFArrayRef list = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
        if (list == NULL) {
            return 0;
        }
        NSArray<NSDictionary *> *windows = CFBridgingRelease(list);
        pid_t ownPid = getpid();
        size_t count = 0;
        for (NSDictionary *info in windows) {
            if (count >= capacity) {
                break;
            }
            NSNumber *layer = info[(__bridge NSString *)kCGWindowLayer];
            NSNumber *owner = info[(__bridge NSString *)kCGWindowOwnerPID];
            NSNumber *alpha = info[(__bridge NSString *)kCGWindowAlpha];
            if (layer == nil || layer.intValue != 0 || owner.intValue == ownPid) {
                continue;
            }
            if (alpha != nil && alpha.doubleValue <= 0.0) {
                continue;
            }
            NSDictionary *boundsInfo = info[(__bridge NSString *)kCGWindowBounds];
            CGRect bounds;
            if (boundsInfo == nil ||
                !CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)boundsInfo, &bounds)) {
                continue;
            }
            rects[count].x = bounds.origin.x;
            rects[count].y = bounds.origin.y;
            rects[count].width = bounds.size.width;
            rects[count].height = bounds.size.height;
            count++;
        }
        return count;
    }
}

// Returns 1 for AC power, 2 for battery or UPS power, 0 when IOKit reports no source.
// `percent` receives the internal battery charge (or -1); `lowPower` receives Low Power Mode.
int32_t auralis_macos_power_source(int32_t *percent, uint8_t *lowPower) {
    @autoreleasepool {
        if (percent != NULL) {
            *percent = -1;
        }
        if (lowPower != NULL) {
            *lowPower = [NSProcessInfo processInfo].isLowPowerModeEnabled ? 1 : 0;
        }
        CFTypeRef info = IOPSCopyPowerSourcesInfo();
        if (info == NULL) {
            return 0;
        }
        int32_t source = 0;
        CFStringRef providing = IOPSGetProvidingPowerSourceType(info);
        if (providing != NULL) {
            if (CFEqual(providing, CFSTR(kIOPMACPowerKey))) {
                source = 1;
            } else if (CFEqual(providing, CFSTR(kIOPMBatteryPowerKey)) ||
                       CFEqual(providing, CFSTR(kIOPMUPSPowerKey))) {
                source = 2;
            }
        }
        CFArrayRef sources = IOPSCopyPowerSourcesList(info);
        if (sources != NULL) {
            for (CFIndex index = 0; index < CFArrayGetCount(sources); index++) {
                CFDictionaryRef description =
                    IOPSGetPowerSourceDescription(info, CFArrayGetValueAtIndex(sources, index));
                if (description == NULL) {
                    continue;
                }
                CFStringRef type = CFDictionaryGetValue(description, CFSTR(kIOPSTypeKey));
                if (type == NULL || !CFEqual(type, CFSTR(kIOPSInternalBatteryType))) {
                    continue;
                }
                CFNumberRef current = CFDictionaryGetValue(description, CFSTR(kIOPSCurrentCapacityKey));
                CFNumberRef maximum = CFDictionaryGetValue(description, CFSTR(kIOPSMaxCapacityKey));
                int currentValue = 0;
                int maximumValue = 0;
                if (current != NULL && maximum != NULL &&
                    CFNumberGetValue(current, kCFNumberIntType, &currentValue) &&
                    CFNumberGetValue(maximum, kCFNumberIntType, &maximumValue) && maximumValue > 0 &&
                    percent != NULL) {
                    long value = lround((double)currentValue * 100.0 / (double)maximumValue);
                    *percent = (int32_t)(value < 0 ? 0 : (value > 100 ? 100 : value));
                }
                break;
            }
            CFRelease(sources);
        }
        CFRelease(info);
        return source;
    }
}

// Resident size, physical footprint and peak resident size from task_info(TASK_VM_INFO);
// user plus system CPU time from getrusage. Returns 0 if either query fails.
int32_t auralis_macos_process_metrics(uint64_t *resident,
                                      uint64_t *footprint,
                                      uint64_t *residentPeak,
                                      uint64_t *cpuMs) {
    task_vm_info_data_t info;
    mach_msg_type_number_t count = TASK_VM_INFO_COUNT;
    if (task_info(mach_task_self(), TASK_VM_INFO, (task_info_t)&info, &count) != KERN_SUCCESS) {
        return 0;
    }
    struct rusage usage;
    if (getrusage(RUSAGE_SELF, &usage) != 0) {
        return 0;
    }
    if (resident != NULL) {
        *resident = info.resident_size;
    }
    if (footprint != NULL) {
        *footprint = info.phys_footprint;
    }
    if (residentPeak != NULL) {
        *residentPeak = info.resident_size_peak;
    }
    if (cpuMs != NULL) {
        uint64_t user = (uint64_t)usage.ru_utime.tv_sec * 1000u + (uint64_t)usage.ru_utime.tv_usec / 1000u;
        uint64_t system = (uint64_t)usage.ru_stime.tv_sec * 1000u + (uint64_t)usage.ru_stime.tv_usec / 1000u;
        *cpuMs = user + system;
    }
    return 1;
}

void auralis_macos_set_reduce_motion(uint8_t enabled) {
    gAuralisReduceMotion = enabled != 0;
}

// MARK: - Studio window

// Content view that accepts a dropped video file and hands its path to the Studio.
@interface AuralisDropView : NSView
@property(nonatomic, copy) void (^onDropPath)(NSString *path);
@end

@implementation AuralisDropView
- (instancetype)initWithFrame:(NSRect)frameRect {
    self = [super initWithFrame:frameRect];
    if (self != nil) {
        [self registerForDraggedTypes:@[NSPasteboardTypeFileURL]];
    }
    return self;
}

- (NSURL *)videoURLFromDrag:(id<NSDraggingInfo>)sender {
    NSArray<NSURL *> *urls = [sender.draggingPasteboard readObjectsForClasses:@[[NSURL class]]
                                                                      options:@{NSPasteboardURLReadingFileURLsOnlyKey : @YES}];
    NSURL *url = urls.firstObject;
    return AuralisIsVideoURL(url) ? url : nil;
}

- (NSDragOperation)draggingEntered:(id<NSDraggingInfo>)sender {
    return [self videoURLFromDrag:sender] != nil ? NSDragOperationCopy : NSDragOperationNone;
}

- (BOOL)performDragOperation:(id<NSDraggingInfo>)sender {
    NSURL *url = [self videoURLFromDrag:sender];
    if (url == nil || self.onDropPath == nil) {
        return NO;
    }
    self.onDropPath(url.path);
    return YES;
}
@end

@interface AuralisStudioController : NSObject <NSWindowDelegate>
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) NSPopUpButton *displayPopup;
@property(nonatomic, strong) NSTextField *pathField;
@property(nonatomic, strong) NSView *previewView;
@property(nonatomic, strong) NSTextField *previewPlaceholder;
@property(nonatomic, strong) NSButton *stopPreviewButton;
@property(nonatomic, strong) NSButton *pauseButton;
@property(nonatomic, strong) NSButton *stopButton;
@property(nonatomic, strong) NSButton *muteCheckbox;
@property(nonatomic, strong) NSButton *loopCheckbox;
@property(nonatomic, strong) NSSegmentedControl *scaleControl;
@property(nonatomic, strong) NSTextField *volumeLabel;
@property(nonatomic, strong) NSTextField *speedLabel;
@property(nonatomic, strong) NSButton *fullscreenCheckbox;
@property(nonatomic, strong) NSButton *batteryCheckbox;
@property(nonatomic, strong) NSButton *loginCheckbox;
@property(nonatomic, strong) NSButton *motionCheckbox;
@property(nonatomic, strong) NSTextField *statusLabel;
@property(nonatomic, strong) NSTextField *detailsLabel;
@property(nonatomic, strong) NSTextField *resourcesLabel;
@property(nonatomic, strong) AVQueuePlayer *previewPlayer;
@property(nonatomic, strong) AVPlayerLooper *previewLooper;
@property(nonatomic, strong) AVPlayerLayer *previewLayer;
- (void)stopPreviewPlayer;
@end

static const CGFloat AuralisGutter = 24.0;
static const CGFloat AuralisPreviewWidth = 512.0;

static NSColor *AuralisGraphite(void) {
    return [NSColor colorWithCalibratedWhite:0.11 alpha:1.0];
}

static NSTextField *AuralisLabel(NSString *text, BOOL semibold, BOOL secondary) {
    NSTextField *label = [NSTextField labelWithString:text];
    label.font = semibold ? [NSFont systemFontOfSize:NSFont.systemFontSize weight:NSFontWeightSemibold]
                          : [NSFont systemFontOfSize:NSFont.systemFontSize];
    label.textColor = secondary ? NSColor.secondaryLabelColor : NSColor.labelColor;
    label.lineBreakMode = NSLineBreakByTruncatingTail;
    label.selectable = NO;
    return label;
}

static NSStackView *AuralisRow(NSArray<NSView *> *views) {
    NSStackView *row = [NSStackView stackViewWithViews:views];
    row.orientation = NSUserInterfaceLayoutOrientationHorizontal;
    row.alignment = NSLayoutAttributeCenterY;
    row.spacing = 8.0;
    return row;
}

@implementation AuralisStudioController
- (NSButton *)button:(NSString *)title action:(SEL)action {
    NSButton *button = [NSButton buttonWithTitle:title target:self action:action];
    button.bezelStyle = NSBezelStyleRounded;
    return button;
}

- (NSButton *)checkbox:(NSString *)title action:(SEL)action {
    return [NSButton checkboxWithTitle:title target:self action:action];
}

- (NSButton *)stepButton:(NSString *)glyph accessibilityLabel:(NSString *)name action:(SEL)action {
    NSButton *button = [self button:glyph action:action];
    button.accessibilityLabel = name;
    button.toolTip = name;
    [button.widthAnchor constraintEqualToConstant:32.0].active = YES;
    return button;
}

- (void)build {
    NSWindow *window = [[NSWindow alloc]
        initWithContentRect:NSMakeRect(0.0, 0.0, AuralisPreviewWidth + 2.0 * AuralisGutter, 760.0)
                  styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable
                    backing:NSBackingStoreBuffered
                      defer:NO];
    window.title = @"Auralis Studio";
    window.releasedWhenClosed = NO;
    window.delegate = self;
    window.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
    window.backgroundColor = AuralisGraphite();
    window.tabbingMode = NSWindowTabbingModeDisallowed;

    AuralisDropView *content = [[AuralisDropView alloc] initWithFrame:window.contentView.bounds];
    __weak AuralisStudioController *weakSelf = self;
    content.onDropPath = ^(NSString *path) {
        // Dropping only fills the field; the preview starts when the user selects Preview.
        weakSelf.pathField.stringValue = path;
    };
    window.contentView = content;

    // Wallpaper
    self.displayPopup = [[NSPopUpButton alloc] initWithFrame:NSZeroRect pullsDown:NO];
    self.displayPopup.target = self;
    self.displayPopup.action = @selector(selectDisplay:);
    self.displayPopup.accessibilityLabel = @"Display";
    NSTextField *displayLabel = AuralisLabel(@"Display", NO, YES);
    [displayLabel.widthAnchor constraintEqualToConstant:72.0].active = YES;

    self.pathField = [NSTextField textFieldWithString:@""];
    self.pathField.placeholderString = @"Choose or drop a video file";
    self.pathField.accessibilityLabel = @"Video file";
    self.pathField.lineBreakMode = NSLineBreakByTruncatingHead;
    [self.pathField setContentHuggingPriority:NSLayoutPriorityDefaultLow
                               forOrientation:NSLayoutConstraintOrientationHorizontal];
    NSTextField *fileLabel = AuralisLabel(@"Video file", NO, YES);
    [fileLabel.widthAnchor constraintEqualToConstant:72.0].active = YES;
    NSButton *browse = [self button:@"Browse…" action:@selector(browse:)];
    NSButton *nextRecent = [self button:@"Next recent" action:@selector(nextRecent:)];

    // Preview
    self.previewView = [[NSView alloc] initWithFrame:NSZeroRect];
    self.previewView.wantsLayer = YES;
    self.previewView.layer.backgroundColor = [NSColor colorWithCalibratedWhite:0.05 alpha:1.0].CGColor;
    self.previewView.layer.cornerRadius = 8.0;
    self.previewView.layer.masksToBounds = YES;
    self.previewView.translatesAutoresizingMaskIntoConstraints = NO;
    [self.previewView.widthAnchor constraintEqualToConstant:AuralisPreviewWidth].active = YES;
    [self.previewView.heightAnchor constraintEqualToConstant:AuralisPreviewWidth * 9.0 / 16.0].active = YES;
    self.previewView.accessibilityLabel = @"Video preview";
    self.previewView.accessibilityRole = NSAccessibilityImageRole;
    self.previewPlaceholder = AuralisLabel(@"Preview is stopped to save resources", NO, YES);
    self.previewPlaceholder.translatesAutoresizingMaskIntoConstraints = NO;
    [self.previewView addSubview:self.previewPlaceholder];
    [self.previewPlaceholder.centerXAnchor constraintEqualToAnchor:self.previewView.centerXAnchor].active = YES;
    [self.previewPlaceholder.centerYAnchor constraintEqualToAnchor:self.previewView.centerYAnchor].active = YES;
    NSButton *preview = [self button:@"Preview" action:@selector(preview:)];
    self.stopPreviewButton = [self button:@"Stop preview" action:@selector(stopPreview:)];
    self.stopPreviewButton.enabled = NO;

    // Wallpaper actions
    NSButton *apply = [self button:@"Set as wallpaper" action:@selector(apply:)];
    apply.keyEquivalent = @"\r";
    self.pauseButton = [self button:@"Pause wallpaper" action:@selector(togglePause:)];
    self.stopButton = [self button:@"Stop wallpaper" action:@selector(stopWallpaper:)];

    // Playback
    self.muteCheckbox = [self checkbox:@"Mute audio" action:@selector(setMuted:)];
    self.loopCheckbox = [self checkbox:@"Loop video" action:@selector(setLoop:)];
    self.scaleControl = [NSSegmentedControl segmentedControlWithLabels:@[@"Fill", @"Fit"]
                                                          trackingMode:NSSegmentSwitchTrackingSelectOne
                                                                target:self
                                                                action:@selector(setScale:)];
    self.scaleControl.accessibilityLabel = @"Scale";
    self.scaleControl.selectedSegment = 0;
    NSTextField *scaleLabel = AuralisLabel(@"Scale", NO, YES);
    self.volumeLabel = AuralisLabel(@"Volume 0%", NO, NO);
    [self.volumeLabel.widthAnchor constraintGreaterThanOrEqualToConstant:88.0].active = YES;
    self.volumeLabel.alignment = NSTextAlignmentCenter;
    self.speedLabel = AuralisLabel(@"Speed 1.00×", NO, NO);
    [self.speedLabel.widthAnchor constraintGreaterThanOrEqualToConstant:88.0].active = YES;
    self.speedLabel.alignment = NSTextAlignmentCenter;
    NSButton *volumeDown = [self stepButton:@"−" accessibilityLabel:@"Volume down" action:@selector(volumeDown:)];
    NSButton *volumeUp = [self stepButton:@"+" accessibilityLabel:@"Volume up" action:@selector(volumeUp:)];
    NSButton *speedDown = [self stepButton:@"−" accessibilityLabel:@"Speed down" action:@selector(speedDown:)];
    NSButton *speedUp = [self stepButton:@"+" accessibilityLabel:@"Speed up" action:@selector(speedUp:)];

    // Behavior
    self.fullscreenCheckbox = [self checkbox:@"Pause while a full-screen app is open"
                                      action:@selector(setPauseOnFullscreen:)];
    self.batteryCheckbox = [self checkbox:@"Pause on battery power" action:@selector(setPauseOnBattery:)];
    self.loginCheckbox = [self checkbox:@"Launch at login" action:@selector(setLaunchAtLogin:)];
    self.motionCheckbox = [self checkbox:@"Reduce motion" action:@selector(setReduceMotion:)];

    // Status
    self.statusLabel = AuralisLabel(@"No wallpaper running", YES, NO);
    self.detailsLabel = AuralisLabel(@"", NO, YES);
    self.resourcesLabel = AuralisLabel(@"", NO, YES);
    NSTextField *closeHint = AuralisLabel(@"Closing this window keeps the wallpaper running.", NO, YES);

    NSArray<NSView *> *sections = @[
        AuralisLabel(@"Wallpaper", YES, NO),
        AuralisRow(@[displayLabel, self.displayPopup]),
        AuralisRow(@[fileLabel, self.pathField, browse, nextRecent]),
        AuralisLabel(@"Preview", YES, NO),
        self.previewView,
        AuralisRow(@[preview, self.stopPreviewButton]),
        AuralisRow(@[apply, self.pauseButton, self.stopButton]),
        AuralisLabel(@"Playback", YES, NO),
        AuralisRow(@[self.muteCheckbox, self.loopCheckbox, scaleLabel, self.scaleControl]),
        AuralisRow(@[volumeDown, self.volumeLabel, volumeUp, speedDown, self.speedLabel, speedUp]),
        AuralisLabel(@"Behavior", YES, NO),
        self.fullscreenCheckbox,
        self.batteryCheckbox,
        self.loginCheckbox,
        self.motionCheckbox,
        self.statusLabel,
        self.detailsLabel,
        self.resourcesLabel,
        closeHint,
    ];
    NSStackView *stack = [NSStackView stackViewWithViews:sections];
    stack.orientation = NSUserInterfaceLayoutOrientationVertical;
    stack.alignment = NSLayoutAttributeLeading;
    stack.spacing = 8.0;
    stack.edgeInsets = NSEdgeInsetsMake(AuralisGutter, AuralisGutter, AuralisGutter, AuralisGutter);
    // 8 px rhythm: 8 inside a group, 24 before each section heading and the status block.
    NSArray<NSView *> *groupEnds = @[sections[2], sections[6], sections[9], sections[14]];
    for (NSView *view in groupEnds) {
        [stack setCustomSpacing:24.0 afterView:view];
    }
    [stack setCustomSpacing:16.0 afterView:sections[5]];
    stack.translatesAutoresizingMaskIntoConstraints = NO;
    [content addSubview:stack];
    [stack.leadingAnchor constraintEqualToAnchor:content.leadingAnchor].active = YES;
    [stack.trailingAnchor constraintEqualToAnchor:content.trailingAnchor].active = YES;
    [stack.topAnchor constraintEqualToAnchor:content.topAnchor].active = YES;
    [stack.bottomAnchor constraintEqualToAnchor:content.bottomAnchor].active = YES;
    for (NSView *view in @[sections[2], self.statusLabel, self.detailsLabel, self.resourcesLabel]) {
        [view.widthAnchor constraintEqualToConstant:AuralisPreviewWidth].active = YES;
    }

    // Size the window to its content so the 24 px gutters hold on every edge.
    [content layoutSubtreeIfNeeded];
    [window setContentSize:stack.fittingSize];
    self.window = window;
    [window center];
}

- (void)queue:(int32_t)action value:(int32_t)value {
    AuralisQueueAction(action, value, nil);
}

- (NSString *)pathText {
    return [self.pathField.stringValue stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
}

- (void)selectDisplay:(id)sender {
    (void)sender;
    [self queue:AuralisActionSelectDisplay value:(int32_t)self.displayPopup.indexOfSelectedItem];
}

- (void)browse:(id)sender {
    (void)sender;
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = YES;
    panel.canChooseDirectories = NO;
    panel.allowsMultipleSelection = NO;
    panel.allowedContentTypes = AuralisVideoTypes();
    panel.prompt = @"Choose";
    __weak AuralisStudioController *weakSelf = self;
    [panel beginSheetModalForWindow:self.window
                  completionHandler:^(NSModalResponse result) {
                      if (result == NSModalResponseOK && panel.URL != nil) {
                          weakSelf.pathField.stringValue = panel.URL.path;
                      }
                  }];
}

- (void)nextRecent:(id)sender {
    (void)sender;
    [self queue:AuralisActionNextRecent value:0];
}

- (void)preview:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionPreview, 0, [self pathText]);
}

- (void)stopPreview:(id)sender {
    (void)sender;
    [self queue:AuralisActionStopPreview value:0];
}

- (void)apply:(id)sender {
    (void)sender;
    AuralisQueueAction(AuralisActionApply, 0, [self pathText]);
}

- (void)togglePause:(id)sender {
    (void)sender;
    [self queue:AuralisActionTogglePause value:0];
}

- (void)stopWallpaper:(id)sender {
    (void)sender;
    [self queue:AuralisActionStop value:0];
}

- (void)setMuted:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetMuted value:self.muteCheckbox.state == NSControlStateValueOn ? 1 : 0];
}

- (void)setLoop:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetLoop value:self.loopCheckbox.state == NSControlStateValueOn ? 1 : 0];
}

- (void)setScale:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetAspect value:self.scaleControl.selectedSegment == 1 ? 1 : 0];
}

- (void)volumeDown:(id)sender {
    (void)sender;
    [self queue:AuralisActionStepVolume value:-1];
}

- (void)volumeUp:(id)sender {
    (void)sender;
    [self queue:AuralisActionStepVolume value:1];
}

- (void)speedDown:(id)sender {
    (void)sender;
    [self queue:AuralisActionStepSpeed value:-1];
}

- (void)speedUp:(id)sender {
    (void)sender;
    [self queue:AuralisActionStepSpeed value:1];
}

- (void)setPauseOnFullscreen:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetPauseOnFullscreen
          value:self.fullscreenCheckbox.state == NSControlStateValueOn ? 1 : 0];
}

- (void)setPauseOnBattery:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetPauseOnBattery value:self.batteryCheckbox.state == NSControlStateValueOn ? 1 : 0];
}

- (void)setLaunchAtLogin:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetLaunchAtLogin value:self.loginCheckbox.state == NSControlStateValueOn ? 1 : 0];
}

- (void)setReduceMotion:(id)sender {
    (void)sender;
    [self queue:AuralisActionSetReduceMotion value:self.motionCheckbox.state == NSControlStateValueOn ? 1 : 0];
}

- (void)stopPreviewPlayer {
    [self.previewPlayer pause];
    [self.previewLooper disableLooping];
    [self.previewPlayer removeAllItems];
    [self.previewLayer removeFromSuperlayer];
    self.previewLooper = nil;
    self.previewPlayer = nil;
    self.previewLayer = nil;
    self.previewPlaceholder.hidden = NO;
    self.stopPreviewButton.enabled = NO;
}

- (void)windowWillClose:(NSNotification *)notification {
    (void)notification;
    // Closing the Studio never stops the wallpaper; it only releases the preview decoder.
    [self stopPreviewPlayer];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    [self queue:AuralisActionStudioClosed value:0];
}
@end

static void AuralisSetCheckbox(NSButton *checkbox, uint8_t on) {
    NSControlStateValue state = on ? NSControlStateValueOn : NSControlStateValueOff;
    if (checkbox.state != state) {
        checkbox.state = state;
    }
}

static void AuralisStudioClose(void) {
    AuralisStudioController *studio = gAuralisStudio;
    if (studio == nil) {
        return;
    }
    [studio stopPreviewPlayer];
    studio.window.delegate = nil;
    [studio.window close];
    gAuralisStudio = nil;
}

void auralis_macos_studio_open(void) {
    @autoreleasepool {
        if (gAuralisStudio == nil) {
            gAuralisStudio = [[AuralisStudioController alloc] init];
            [gAuralisStudio build];
        }
        NSApplication *application = [NSApplication sharedApplication];
        // An LSUIElement app has no Dock icon and cannot normally take focus; become a regular
        // app while the Studio is open, then return to accessory when it closes.
        [application setActivationPolicy:NSApplicationActivationPolicyRegular];
        gAuralisStudio.window.animationBehavior =
            AuralisShouldReduceMotion() ? NSWindowAnimationBehaviorNone : NSWindowAnimationBehaviorDefault;
        [gAuralisStudio.window makeKeyAndOrderFront:nil];
        [application activateIgnoringOtherApps:YES];
    }
}

void auralis_macos_studio_set_displays(const char *labels, int32_t selected) {
    @autoreleasepool {
        AuralisStudioController *studio = gAuralisStudio;
        if (studio == nil) {
            return;
        }
        NSString *joined = AuralisString(labels);
        NSArray<NSString *> *titles = joined.length > 0 ? [joined componentsSeparatedByString:@"\n"] : @[];
        [studio.displayPopup removeAllItems];
        [studio.displayPopup addItemsWithTitles:titles];
        if (selected >= 0 && selected < (int32_t)studio.displayPopup.numberOfItems) {
            [studio.displayPopup selectItemAtIndex:selected];
        }
        studio.displayPopup.enabled = titles.count > 1;
    }
}

void auralis_macos_studio_set_path(const char *path) {
    @autoreleasepool {
        gAuralisStudio.pathField.stringValue = AuralisString(path);
    }
}

void auralis_macos_studio_update(const AuralisStudioState *state) {
    @autoreleasepool {
        AuralisStudioController *studio = gAuralisStudio;
        if (studio == nil || state == NULL) {
            return;
        }
        studio.statusLabel.stringValue = AuralisString(state->status);
        studio.detailsLabel.stringValue = AuralisString(state->details);
        studio.resourcesLabel.stringValue = AuralisString(state->resources);
        studio.volumeLabel.stringValue = AuralisString(state->volumeLabel);
        studio.speedLabel.stringValue = AuralisString(state->speedLabel);
        studio.pauseButton.title = AuralisString(state->pauseLabel);
        studio.pauseButton.enabled = state->controlsEnabled != 0;
        studio.stopButton.enabled = state->controlsEnabled != 0;
        studio.stopPreviewButton.enabled = state->previewActive != 0;
        AuralisSetCheckbox(studio.muteCheckbox, state->muted);
        AuralisSetCheckbox(studio.loopCheckbox, state->loopPlayback);
        studio.scaleControl.selectedSegment = state->fitAspect ? 1 : 0;
        AuralisSetCheckbox(studio.fullscreenCheckbox, state->pauseOnFullscreen);
        AuralisSetCheckbox(studio.batteryCheckbox, state->pauseOnBattery);
        AuralisSetCheckbox(studio.loginCheckbox, state->launchAtLogin);
        AuralisSetCheckbox(studio.motionCheckbox, state->reduceMotion);
    }
}

int32_t auralis_macos_studio_start_preview(const char *path, uint8_t fitAspect) {
    @autoreleasepool {
        AuralisStudioController *studio = gAuralisStudio;
        if (studio == nil || path == NULL) {
            return 0;
        }
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        if (url == nil) {
            return 0;
        }
        [studio stopPreviewPlayer];
        AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];
        AVQueuePlayer *player = [AVQueuePlayer queuePlayerWithItems:@[]];
        player.muted = YES;
        player.allowsExternalPlayback = NO;
        AVPlayerLooper *looper = [AVPlayerLooper playerLooperWithPlayer:player templateItem:item];
        AVPlayerLayer *layer = [AVPlayerLayer playerLayerWithPlayer:player];
        layer.frame = studio.previewView.bounds;
        layer.autoresizingMask = kCALayerWidthSizable | kCALayerHeightSizable;
        layer.videoGravity = fitAspect ? AVLayerVideoGravityResizeAspect : AVLayerVideoGravityResizeAspectFill;
        [studio.previewView.layer addSublayer:layer];
        studio.previewPlayer = player;
        studio.previewLooper = looper;
        studio.previewLayer = layer;
        studio.previewPlaceholder.hidden = YES;
        studio.stopPreviewButton.enabled = YES;
        [player play];
        return 1;
    }
}

void auralis_macos_studio_stop_preview(void) {
    @autoreleasepool {
        [gAuralisStudio stopPreviewPlayer];
    }
}

// Returns 1 and writes a message when the running preview failed to load or play.
int32_t auralis_macos_studio_preview_error(char *message, size_t capacity) {
    @autoreleasepool {
        AuralisStudioController *studio = gAuralisStudio;
        if (message != NULL && capacity > 0) {
            message[0] = '\0';
        }
        if (studio.previewPlayer == nil) {
            return 0;
        }
        NSError *error = nil;
        if (studio.previewLooper.status == AVPlayerLooperStatusFailed) {
            error = studio.previewLooper.error;
        } else if (studio.previewPlayer.currentItem.status == AVPlayerItemStatusFailed) {
            error = studio.previewPlayer.currentItem.error;
        } else if (studio.previewPlayer.status == AVPlayerStatusFailed) {
            error = studio.previewPlayer.error;
        } else {
            return 0;
        }
        if (message != NULL && capacity > 0) {
            const char *text = error.localizedDescription.UTF8String ?: "AVFoundation could not play this video";
            snprintf(message, capacity, "%s", text);
        }
        return 1;
    }
}
