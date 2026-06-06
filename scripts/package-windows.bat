@echo off
chcp 65001 >nul
setlocal enabledelayedexpansion

echo ============================================
echo  Sakichan Windows 发布包打包脚本
echo ============================================
echo.
echo DLL 和数据文件与 exe 放同目录，双击即用
echo.

set "MSYS2=c:\msys64\ucrt64"
set "RELEASE_DIR=%~dp0target\release"
set "OUTPUT=%~dp0Sakichan-Windows"

if not exist "%RELEASE_DIR%\sakichan-gtk.exe" (
    echo [错误] 找不到编译产物: %RELEASE_DIR%\sakichan-gtk.exe
    echo 请先运行: cargo build --release -p sakichan-gtk
    pause
    exit /b 1
)

echo [1/5] 创建输出目录...
if exist "%OUTPUT%" rmdir /s /q "%OUTPUT%"
mkdir "%OUTPUT%"
mkdir "%OUTPUT%\lib\gdk-pixbuf-2.0\2.10.0\loaders"
mkdir "%OUTPUT%\share\icons"
mkdir "%OUTPUT%\share\glib-2.0\schemas"
mkdir "%OUTPUT%\share\gtk-4.0"
mkdir "%OUTPUT%\etc\fonts"

echo [2/5] 复制可执行文件...
copy "%RELEASE_DIR%\sakichan-gtk.exe" "%OUTPUT%\" >nul

echo [3/5] 查找并复制依赖 DLL（与 exe 同目录）...
set "PATH=%MSYS2%\bin;%PATH%"

REM 用 ntldd -R 递归分析所有依赖
%MSYS2%\bin\ntldd.exe -R "%RELEASE_DIR%\sakichan-gtk.exe" 2>nul | find "%MSYS2%" > "%TEMP%\dll_list.txt"

for /f "tokens=4 delims= " %%d in (%TEMP%\dll_list.txt) do (
    if not exist "%OUTPUT%\%%~nxd" (
        copy "%%d" "%OUTPUT%\" >nul 2>nul
    )
)

echo [4/5] 复制 GTK 运行时数据...

REM 图标主题 (GTK4 必需)
xcopy /e /i /q "%MSYS2%\share\icons\Adwaita" "%OUTPUT%\share\icons\Adwaita\" >nul 2>nul
xcopy /e /i /q "%MSYS2%\share\icons\hicolor" "%OUTPUT%\share\icons\hicolor\" >nul 2>nul

REM GLib schemas
copy "%MSYS2%\share\glib-2.0\schemas\gschemas.compiled" "%OUTPUT%\share\glib-2.0\schemas\" >nul 2>nul

REM GDK pixbuf loaders + cache
copy "%MSYS2%\lib\gdk-pixbuf-2.0\2.10.0\loaders\*.dll" "%OUTPUT%\lib\gdk-pixbuf-2.0\2.10.0\loaders\" >nul 2>nul
%MSYS2%\bin\gdk-pixbuf-query-loaders.exe "%OUTPUT%\lib\gdk-pixbuf-2.0\2.10.0\loaders\*.dll" > "%OUTPUT%\lib\gdk-pixbuf-2.0\2.10.0\loaders.cache" 2>nul

REM GTK4 emoji 数据
xcopy /e /i /q "%MSYS2%\share\gtk-4.0\emoji" "%OUTPUT%\share\gtk-4.0\emoji\" >nul 2>nul

REM 字体配置
copy "%MSYS2%\etc\fonts\fonts.conf" "%OUTPUT%\etc\fonts\" >nul 2>nul
if exist "%MSYS2%\etc\fonts\conf.d" (
    mkdir "%OUTPUT%\etc\fonts\conf.d"
    xcopy /e /i /q "%MSYS2%\etc\fonts\conf.d" "%OUTPUT%\etc\fonts\conf.d\" >nul 2>nul
)

echo [5/5] 统计打包结果...
for /f %%i in ('dir /a-d /s /b "%OUTPUT%" 2^>nul ^| find /v /c ""') do set FILECOUNT=%%i
for /f "tokens=3" %%i in ('dir /-c "%OUTPUT%" 2^>nul ^| find "File(s)"') do set TOTALSIZE=%%i

echo.
echo ============================================
echo  打包完成！
echo.
echo  输出目录: %OUTPUT%
echo  文件数:   %FILECOUNT%
echo  总大小:   %TOTALSIZE% 字节
echo.
echo  使用方法:
echo    1. 把 Sakichan-Windows 文件夹整个复制给用户
echo    2. 用户双击 sakichan-gtk.exe 即可运行
echo    3. 无需安装任何组件
echo ============================================
echo.
pause
