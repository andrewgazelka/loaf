# Loaf MVP Status

## What Works

- **Zig core library** - SQLite-backed filesystem operations (create, read, write, delete, rename, symlinks)
- **Swift FSKit extension** - Implements `FSUnaryFileSystem`, `FSVolume`, `FSItem` protocols
- **CLI commands** - `loaf init`, `loaf diff`, `loaf accept`, `loaf reject`, `loaf ls`, `loaf cat`, `loaf tree`, `loaf run`
- **Build script** (`scripts/build.sh`) - Builds, signs with Developer ID, notarizes, and installs to /Applications
- **Notarization** - App is properly notarized and accepted by Gatekeeper (`spctl -a -v` returns "Notarized Developer ID")
- **Extension registration** - `pluginkit` shows extension as enabled (`+`)

## What Doesn't Work

### FSKit Extension Toggle Bounces Back

**The Problem:**
In System Settings → General → Login Items & Extensions → File System Extensions, the "FSKit Modules" toggle for Loaf bounces back to OFF when clicked.

**Mount Error:**
```
Module com.loaf.app.extension is disabled!
mount: Unable to invoke task
```

**What We've Tried:**
1. ✅ Signed with Developer ID Application certificate
2. ✅ Notarized with Apple
3. ✅ Hardened runtime enabled
4. ✅ Secure timestamp in signature
5. ✅ Removed `get-task-allow` entitlement (debug entitlement)
6. ✅ Added `com.apple.developer.fskit.fsmodule` entitlement to extension
7. ✅ Added `com.apple.security.temporary-exception.mach-lookup.global-name` for `com.apple.filesystems.fskitd` to main app
8. ✅ Cleaned up duplicate registrations
9. ✅ Re-registered with `pluginkit -a`
10. ✅ Restarted System Settings

**Logs show no rejection reason** - just normal app lifecycle events, no explicit deny/reject/block messages.

### Potential Causes

1. **macOS 26 (Tahoe) beta bug** - FSKit is relatively new (macOS 15.4+) and may have issues in beta
2. **Extension cache** - May require logout/login or reboot to clear
3. **Per-user setting** - FSKit extension enable is per-user, not system-wide
4. **Missing entitlement or provisioning** - Developer ID signing for FSKit may need special provisioning

## Environment

- macOS 26.1 Tahoe (25B78) - **beta**
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
3. **Wait for macOS 26 stable** - May be a beta bug
4. **File Apple Feedback** - FB# for FSKit toggle issue

## References

- [FSKitSample](https://github.com/KhaosT/FSKitSample) - Reference implementation (deps/FSKitSample)
- [Apple FSKit Forums](https://developer.apple.com/forums/tags/fskit)
- [Eclectic Light FSKit Overview](https://eclecticlight.co/2024/06/26/how-file-systems-can-change-in-sequoia-with-fskit/)
