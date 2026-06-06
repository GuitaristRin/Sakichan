#Requires -Version 5.0
# Sakichan Windows packaging script
# Usage: .\package-windows.ps1
# Output: ..\Sakichan-Windows\

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$MingwDir = "C:\msys64\ucrt64"
$NtlddPath = "C:\msys64\mingw64\bin\ntldd.exe"
$ReleaseDir = "$ProjectRoot\target\release"
$OutputDir = "$ProjectRoot\Sakichan-Windows"

Write-Host "Sakichan Windows Release Packager" -ForegroundColor Cyan
Write-Host ""

# Check build artifact
if (-not (Test-Path "$ReleaseDir\sakichan-gtk.exe")) {
    Write-Host "[ERROR] Build artifact not found: $ReleaseDir\sakichan-gtk.exe" -ForegroundColor Red
    Write-Host "Run: cargo build --release -p sakichan-gtk" -ForegroundColor Yellow
    exit 1
}

Write-Host "[1/5] Creating output directory..." -ForegroundColor Green
if (Test-Path $OutputDir) { Remove-Item -Recurse -Force $OutputDir }
New-Item -ItemType Directory -Path "$OutputDir\lib\gdk-pixbuf-2.0\2.10.0\loaders" | Out-Null
New-Item -ItemType Directory -Path "$OutputDir\share\icons" | Out-Null
New-Item -ItemType Directory -Path "$OutputDir\share\glib-2.0\schemas" | Out-Null
New-Item -ItemType Directory -Path "$OutputDir\share\gtk-4.0" | Out-Null
New-Item -ItemType Directory -Path "$OutputDir\etc\fonts" | Out-Null

Write-Host "[2/5] Copying executable..." -ForegroundColor Green
Copy-Item "$ReleaseDir\sakichan-gtk.exe" "$OutputDir\"

Write-Host "[3/5] Resolving and copying DLL dependencies..." -ForegroundColor Green
$env:Path = "$MingwDir\bin;$env:Path"
$ntlddOutput = & $NtlddPath -R "$ReleaseDir\sakichan-gtk.exe" 2>&1
$dllPaths = $ntlddOutput | Select-String "$MingwDir" | ForEach-Object {
    $line = $_.ToString()
    if ($line -match "=> (.+?) `(") {
        $matches[1].Trim()
    }
} | Select-Object -Unique

$copied = 0
$skipped = 0
foreach ($dll in $dllPaths) {
    $name = Split-Path -Leaf $dll
    $dest = Join-Path $OutputDir $name
    if (-not (Test-Path $dest)) {
        Copy-Item $dll $dest -ErrorAction SilentlyContinue
        if (Test-Path $dest) { $copied++ } else { $skipped++ }
    }
}
Write-Host "  $copied DLLs copied, $skipped skipped" -ForegroundColor Gray

Write-Host "[4/5] Copying GTK runtime data..." -ForegroundColor Green

# Icon themes (required by GTK4)
$icons = @("Adwaita", "hicolor")
foreach ($icon in $icons) {
    $src = "$MingwDir\share\icons\$icon"
    if (Test-Path $src) {
        Copy-Item -Recurse $src "$OutputDir\share\icons\" -ErrorAction SilentlyContinue
        Write-Host "  + share\icons\$icon" -ForegroundColor Gray
    }
}

# GLib schemas
$schemasSrc = "$MingwDir\share\glib-2.0\schemas\gschemas.compiled"
if (Test-Path $schemasSrc) {
    Copy-Item $schemasSrc "$OutputDir\share\glib-2.0\schemas\"
    Write-Host "  + share\glib-2.0\schemas\gschemas.compiled" -ForegroundColor Gray
}

# GDK pixbuf loaders
Copy-Item "$MingwDir\lib\gdk-pixbuf-2.0\2.10.0\loaders\*.dll" "$OutputDir\lib\gdk-pixbuf-2.0\2.10.0\loaders\" -ErrorAction SilentlyContinue
Write-Host "  + lib\gdk-pixbuf-2.0\2.10.0\loaders\*.dll" -ForegroundColor Gray

# Generate loaders.cache
$loaders = Get-ChildItem "$OutputDir\lib\gdk-pixbuf-2.0\2.10.0\loaders\*.dll" | ForEach-Object { $_.FullName }
if ($loaders.Count -gt 0) {
    $gdkQuery = "$MingwDir\bin\gdk-pixbuf-query-loaders.exe"
    if (Test-Path $gdkQuery) {
        $cacheContent = & $gdkQuery $loaders 2>$null
        if ($cacheContent) {
            $cachePath = "$OutputDir\lib\gdk-pixbuf-2.0\2.10.0\loaders.cache"
            $cacheContent | Out-File -FilePath $cachePath -Encoding ASCII
            # Fix absolute paths to relative
            $content = Get-Content $cachePath
            $content -replace [regex]::Escape($OutputDir), "." | Set-Content $cachePath -Encoding ASCII
            Write-Host "  + lib\gdk-pixbuf-2.0\2.10.0\loaders.cache" -ForegroundColor Gray
        }
    }
}

# GTK4 emoji data
$emojiSrc = "$MingwDir\share\gtk-4.0\emoji"
if (Test-Path $emojiSrc) {
    Copy-Item -Recurse $emojiSrc "$OutputDir\share\gtk-4.0\" -ErrorAction SilentlyContinue
    Write-Host "  + share\gtk-4.0\emoji" -ForegroundColor Gray
}

# Font config
$fontsConf = "$MingwDir\etc\fonts\fonts.conf"
if (Test-Path $fontsConf) {
    Copy-Item $fontsConf "$OutputDir\etc\fonts\"
    Write-Host "  + etc\fonts\fonts.conf" -ForegroundColor Gray
}
$confDir = "$MingwDir\etc\fonts\conf.d"
if (Test-Path $confDir) {
    New-Item -ItemType Directory -Path "$OutputDir\etc\fonts\conf.d" | Out-Null
    Copy-Item "$confDir\*" "$OutputDir\etc\fonts\conf.d\" -ErrorAction SilentlyContinue
    Write-Host "  + etc\fonts\conf.d" -ForegroundColor Gray
}

Write-Host "[5/5] Summary..." -ForegroundColor Green
$fileCount = (Get-ChildItem -Recurse -File $OutputDir).Count
$totalSize = (Get-ChildItem -Recurse $OutputDir | Measure-Object -Property Length -Sum).Sum
$sizeMB = [math]::Round($totalSize / 1MB, 1)

Write-Host ""
Write-Host "========================" -ForegroundColor Cyan
Write-Host "  Packaging complete!" -ForegroundColor Cyan
Write-Host "========================" -ForegroundColor Cyan
Write-Host ""
Write-Host "Output: $OutputDir" -ForegroundColor White
Write-Host "Files:  $fileCount" -ForegroundColor White
Write-Host "Size:   $sizeMB MB" -ForegroundColor White
Write-Host ""
Write-Host "Distribute the Sakichan-Windows folder as-is." -ForegroundColor Yellow
Write-Host "Users just double-click sakichan-gtk.exe - no dependencies needed." -ForegroundColor Yellow
Write-Host ""

Read-Host "Press Enter to exit"