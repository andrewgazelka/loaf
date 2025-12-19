# Loaf MVP Status

## What Works

- **Zig core library** - SQLite-backed filesystem operations (create, read, write, delete, rename, symlinks)
- **Swift FSKit extension** - Implements `FSUnaryFileSystem`, `FSVolume`, `FSItem` protocols
- **CLI commands** - `loaf init`, `loaf diff`, `loaf accept`, `loaf reject`, `loaf ls`, `loaf cat`, `loaf tree`, `loaf run`
- **Build script** (`scripts/build.sh`) - Builds, signs with Developer ID, notarizes, and installs to /Applications
- **Notarization** - App is properly notarized and accepted by Gatekeeper (`spctl -a -v` returns "Notarized Developer ID")
- **Extension registration** - `pluginkit` shows extension as enabled (`+`)

## What Doesn't Work

### FSKit Extension Launch Fails with Security Policy Error

**The Problem:**
The extension is registered and enabled, but fails to spawn with `error 163: Security policy issue`.

**Mount Error:**
```
mount: Probing resource: The operation couldn't be completed. (com.apple.extensionKit.errorDomain error 2.)
mount: Unable to invoke task
```

**RunningBoard Log:**
```
start succeeded, info=spawn failed, error=163: Security policy issue
Process start failed with Error Domain=NSPOSIXErrorDomain Code=163 "Unknown error: 163" UserInfo={NSLocalizedDescription=Launchd job spawn failed}
```

**Root Cause Analysis:**
- Error 163 is not documented in Apple's security error codes
- RunningBoard/launchd is refusing to spawn the extension process
- This happens even with proper code signing, notarization, and hardened runtime
- Possibly a macOS 26 Tahoe issue with third-party FSKit extensions

### Toggle Bouncing Issue (SOLVED)

**Original Problem:** FSKit toggle in System Settings bounces back to OFF.

**Solution:** FSKit module enable state is stored in user's Group Container, NOT controlled by pluginkit:
```
~/Library/Group Containers/group.com.apple.fskit.settings/enabledModules.plist
/var/root/Library/Group Containers/group.com.apple.fskit.settings/enabledModules.plist
```

**Workaround:** Add extension ID to root's enabledModules.plist:
```bash
sudo plutil -insert 0 -string "com.loaf.app.extension" /var/root/Library/Group\ Containers/group.com.apple.fskit.settings/enabledModules.plist
sudo pkill -HUP fskitd
```

After this, the "Module is disabled" error changes to the spawn error above.

### Library Loading (FIXED)

**Problem:** Extension linked against `@rpath/libloaf.dylib` but dylib wasn't embedded.

**Solution:** Build script now:
1. Creates `LoafExtension.appex/Contents/Frameworks/`
2. Copies `libloaf.dylib` there
3. Signs dylib with Developer ID
4. Re-signs extension and app

### What We've Tried (for spawn error):
1. ✅ Signed with Developer ID Application certificate
2. ✅ Notarized with Apple
3. ✅ Hardened runtime enabled
4. ✅ Secure timestamp in signature
5. ✅ Added `com.apple.developer.fskit.fsmodule` entitlement to extension
6. ✅ Added `com.apple.security.cs.disable-library-validation` entitlement
7. ✅ Embedded and signed libloaf.dylib in extension's Frameworks folder
8. ✅ Manually enabled in root's enabledModules.plist
9. ✅ Restarted fskitd (`sudo pkill -HUP fskitd`)
10. ❌ Static linking (breaks notarization)

### Potential Causes for Spawn Error

1. **macOS 26 (Tahoe) restriction** - FSKit may have additional restrictions for third-party extensions
2. **Missing undocumented entitlement** - FSKit may require special provisioning for third-party extensions
3. **Sandbox profile issue** - Extension's sandbox may be blocking something required for spawn
4. **AMFI/SIP restriction** - May need to disable SIP for third-party FSKit extensions (not ideal)

## Environment

- macOS 26.1 Tahoe (25B78)
- Xcode 16+
- Zig 0.15+
- Apple Developer Program (Individual)
- Team ID: WJQ6TR5FJS

## Build Commands

```bash
# Full build, sign, notarize, and install
./scripts/build.sh

# Just build Zig library
zig build -Doptimize=ReleaseFast

# Run tests
zig build test
```

## Workarounds to Try

1. **Logout/Login or Reboot** - Clear extension cache
2. **Disable SIP** (invasive):
   ```bash
   # Boot to Recovery Mode (hold Power button)
   # Terminal: csrutil disable
   # Reboot, then:
   sudo systemextensionsctl developer on
   ```
3. **Check Apple Developer Forums** - Others may have encountered this
4. **File Apple Feedback** - FB# for FSKit toggle issue

## References

- [FSKitSample](https://github.com/KhaosT/FSKitSample) - Reference implementation (deps/FSKitSample)
- [Apple FSKit Forums](https://developer.apple.com/forums/tags/fskit)
- [Eclectic Light FSKit Overview](https://eclecticlight.co/2024/06/26/how-file-systems-can-change-in-sequoia-with-fskit/)
