## Downloads

ProjectorHUD is a single program with nothing else to install.

| Platform | File |
|----------|------|
| Windows 10/11 (x64) | `ProjectorHUD-{{TAG}}-windows-x64.zip` |
| Linux (x64, glibc 2.35+) | `ProjectorHUD-{{TAG}}-linux-x64.tar.gz` |
| macOS 11+ (Apple Silicon and Intel) | `ProjectorHUD-{{TAG}}-macos-universal.zip` |

**Windows:** unzip and run `ProjectorHUD.exe`. These builds aren't code-signed
yet, so SmartScreen may warn: choose *More info → Run anyway*.

**Linux:** extract and run `./projectorhud`. Needs an OpenGL-capable graphics
driver and X11 or Wayland.

**macOS:** unzip and move `ProjectorHUD.app` to Applications. These builds
aren't signed with an Apple Developer ID yet, so macOS refuses the first open:
go to *System Settings → Privacy & Security* and choose *Open Anyway*. The
first time it searches for projectors, macOS asks to allow access to the local
network: choose *Allow*.

`SHA256SUMS.txt` lists checksums for every file.
