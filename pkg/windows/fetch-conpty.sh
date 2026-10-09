#!/usr/bin/env bash
# Put Microsoft's ConPTY (conpty.dll + OpenConsole.exe) into the directory
# given, beside tmxr.exe. Windows' built-in ConPTY drops the end of a
# program's DCS, which breaks tmux passthrough (inline images); this one
# keeps it, and portable-pty loads a conpty.dll found beside the program.
# The package is pinned by version and SHA-256, and both files must carry a
# valid Microsoft Corporation signature. Runs in CI's Git Bash on Windows.
set -euo pipefail

version=1.25.260930003
sha256=02b07b349af66d801159bdf9e440d4a1ce78bb951f37fc8609731665afdae7ee
dest=$1

work=$(mktemp -d)
url="https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$version/microsoft.windows.console.conpty.$version.nupkg"
curl -sSfL -o "$work/conpty.nupkg" "$url"
echo "$sha256  $work/conpty.nupkg" | sha256sum -c -
unzip -q "$work/conpty.nupkg" -d "$work/x"
mkdir -p "$dest"
cp "$work/x/runtimes/win-x64/native/conpty.dll" "$work/x/build/native/runtimes/x64/OpenConsole.exe" "$dest/"
for f in conpty.dll OpenConsole.exe; do
  pwsh -NoProfile -Command "
    \$s = Get-AuthenticodeSignature '$(cygpath -w "$dest/$f")'
    if (\$s.Status -ne 'Valid' -or \$s.SignerCertificate.Subject -notmatch '^CN=Microsoft Corporation,') {
      throw '$f: signature ' + \$s.Status + ' by ' + \$s.SignerCertificate.Subject
    }"
done
echo "ConPTY $version in $dest"
