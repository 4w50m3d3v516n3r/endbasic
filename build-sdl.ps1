# Builds the EndBASIC CLI with SDL support in release mode on Windows.
#
# A global CC variable (e.g. set by the Raspberry Pi Pico SDK installer) breaks the `cc` crate
# because it points to a cross-compiler in a path with spaces, so clear it for this process only.

if (Test-Path Env:CC) {
    Remove-Item Env:CC
}

cargo build --release --features=sdl -p endbasic @args
exit $LASTEXITCODE
