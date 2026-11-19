# Verify that a built auralis.exe carries the Auralis icon and version information.
# Usage: powershell -ExecutionPolicy Bypass -File scripts/check-exe-resources.ps1 [path-to-exe]
# Used by CI after the Windows release build; build.rs only warns when no resource compiler
# is available, so this check is what keeps a release from shipping without them.

param([string]$Exe = "target/release/auralis.exe")

$ErrorActionPreference = "Stop"
$file = Get-Item $Exe
$info = $file.VersionInfo
$info | Format-List ProductName, FileDescription, FileVersion, ProductVersion, CompanyName, LegalCopyright, OriginalFilename

$cargoVersion = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$expected = @{
    ProductName      = "Auralis"
    FileDescription  = "Auralis live wallpaper"
    OriginalFilename = "auralis.exe"
}
$failures = @()
foreach ($key in $expected.Keys) {
    if ($info.$key -ne $expected[$key]) { $failures += "$key is '$($info.$key)', expected '$($expected[$key])'" }
}
# ProductVersion is the Cargo version, optionally followed by +build.<N> from CI.
if (-not ($info.ProductVersion -eq $cargoVersion -or $info.ProductVersion -like "$cargoVersion+build.*")) {
    $failures += "ProductVersion is '$($info.ProductVersion)', expected '$cargoVersion[+build.N]'"
}
$numeric = ($cargoVersion -split '[-+]')[0]
if ("$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart)" -ne $numeric) {
    $failures += "Numeric file version is $($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart), expected $numeric"
}

# ExtractAssociatedIcon falls back to a generic icon, so look for the icon group resource itself.
Add-Type -Namespace Auralis -Name Res -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
[DllImport("kernel32.dll")]
public static extern bool FreeLibrary(IntPtr module);
'@
$LOAD_LIBRARY_AS_DATAFILE = 0x2
$RT_GROUP_ICON = [IntPtr]14
$module = [Auralis.Res]::LoadLibraryExW($file.FullName, [IntPtr]::Zero, $LOAD_LIBRARY_AS_DATAFILE)
if ($module -eq [IntPtr]::Zero) { throw "Could not load $($file.FullName) as a data file" }
try {
    if ([Auralis.Res]::FindResourceW($module, [IntPtr]1, $RT_GROUP_ICON) -eq [IntPtr]::Zero) {
        $failures += "Icon group resource 1 is missing"
    }
} finally {
    [void][Auralis.Res]::FreeLibrary($module)
}

if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Host "[FAIL] $_" -ForegroundColor Red }
    exit 1
}
Write-Host "[PASS] $($file.Name) has the Auralis icon and version information." -ForegroundColor Green
