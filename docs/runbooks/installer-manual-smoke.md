# Manual smoke: installer scripts

Verifies that `installer/install.sh` and `installer/install.ps1` install,
upgrade, and uninstall Teramind cleanly against a local fixture host.

## Unix (install.sh)

### Setup

```sh
TMP=$(mktemp -d)
mkdir -p "$TMP/0.0.1"
mkdir -p "$TMP/build/teramind-0.0.1"
cargo build --release
cp target/release/{teramind,teramindd,teramind-hook,teramind-mcp} \
   "$TMP/build/teramind-0.0.1/"
TRIPLE=$(uname -m | sed -e 's/arm64/aarch64/' -e 's/amd64/x86_64/')-$(uname -s | sed -e 's/Linux/unknown-linux-gnu/' -e 's/Darwin/apple-darwin/')
ARCHIVE="teramind-0.0.1-${TRIPLE}.tar.gz"
( cd "$TMP/build" && tar -czf "$TMP/0.0.1/$ARCHIVE" "teramind-0.0.1" )
SUM=$(shasum -a 256 "$TMP/0.0.1/$ARCHIVE" | awk '{print $1}')
echo "$SUM  $ARCHIVE" > "$TMP/0.0.1/teramind-0.0.1-SHA256SUMS"
cat > "$TMP/releases.json" <<EOF
{"latest":"0.0.1","releases":[{"version":"0.0.1","artifacts":{"${TRIPLE}":{"url":"http://127.0.0.1:38080/0.0.1/$ARCHIVE","sha256":"$SUM"}}}]}
EOF
( cd "$TMP" && python3 -m http.server 38080 ) &
SERVER=$!
sleep 1
```

### Install

```sh
TERAMIND_RELEASE_BASE=http://127.0.0.1:38080 \
TERAMIND_INSTALL_ROOT="$TMP/install" \
TERAMIND_BIN_DIR="$TMP/binshadow" \
sh installer/install.sh
```

**Expect:**
- Exit 0.
- `$TMP/install/bin/teramind` exists and is executable.
- `$TMP/binshadow/teramind` is a symlink to the above.

### Self-update no-op

```sh
TERAMIND_RELEASE_INDEX_URL="http://127.0.0.1:38080/releases.json" \
"$TMP/install/bin/teramind" self-update --check-only
```

**Expect:** "already at latest" (because we just installed the only published version).

### Uninstall

```sh
TERAMIND_INSTALL_ROOT="$TMP/install" \
TERAMIND_BIN_SYMLINK="$TMP/binshadow/teramind" \
"$TMP/install/bin/teramind" uninstall --confirm
```

**Expect:** All four binaries + the symlink reported as `[removed]`; data dirs preserved.

### Tear down

```sh
kill $SERVER
rm -rf "$TMP"
```

## Windows (install.ps1)

Same idea, but use `python -m http.server 38080` from a different shell, and:

```pwsh
$env:TERAMIND_RELEASE_BASE = "http://127.0.0.1:38080"
$env:TERAMIND_INSTALL_ROOT = "$env:TEMP\teramind-test"
$env:TERAMIND_NO_MODIFY_PATH = "1"
powershell -ExecutionPolicy Bypass -File installer/install.ps1
```

**Expect:** Same outcomes as the Unix path, modulo the symlink step (Windows uses PATH prepending instead).
