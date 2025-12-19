# Loaf MVP Status

## What Works

- **Zig core library** - SQLite-backed filesystem operations (create, read, write, delete, rename, symlinks)
- **Swift FSKit extension** - Implements `FSUnaryFileSystem`, `FSVolume`, `FSItem` protocols
- **CLI commands** - `loaf init`, `loaf diff`, `loaf accept`, `loaf reject`, `loaf ls`, `loaf cat`, `loaf tree`, `loaf run`
- **Build script** (`scripts/build.sh`) - Builds, signs with Developer ID, notarizes, and installs to /Applications
- **Notarization** - App is properly notarized and accepted by Gatekeeper (`spctl -a -v` returns "Notarized Developer ID")
- **Extension registration** - `pluginkit` shows extension as enabled (`+`)

## What Doesn't Work

### FSKit Third-Party Extensions Are Broken on macOS 26

**This is NOT a Loaf-specific issue.** We tested [FSKitSample](https://github.com/KhaosT/FSKitSample) (Apple's reference implementation) and it fails identically:

```
fskitd: [com.apple.FSKit:default] Hello FSClient! entitlement no
fskitd: [com.apple.FSKit:default] About to get current agent for 501
mount: Unable to invoke task
```

**Key Finding:** The `entitlement no` message indicates fskitd refuses connections from unprivileged clients. This affects ALL third-party FSKit extensions, not just Loaf.

### Known FSKit Issues (from Apple Developer Forums)

Per [FSKit module mount fails with permissions](https://developer.apple.com/forums/thread/788609):

1. **fskitd permission issues** - fskitd doesn't have permissions to access real disks when initiated from mount utility
2. **DiskArbitration blocking** - Third-party FSKit modules are deprioritized after KEXTs, effectively blocking them
3. **UUID registration instability** - lsd occasionally re-registers extensions with different UUIDs
4. **Multiple bugs being fixed** - Apple engineer (Kevin Elliott, DTS) stated in July 2025: "more bugs have been found so you're going to need to wait for more fixes"

### Specific Errors We See

**Mount Error:**
```
mount: Probing resource: The operation couldn't be completed. (com.apple.extensionKit.errorDomain error 2.)
mount: Unable to invoke task
```

**fskitd Log:**
```
fskitd: [com.apple.FSKit:default] Incomming connection, entitled 0
fskitd: [com.apple.FSKit:default] Hello FSClient! entitlement no
```

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

### Library Loading (FIXED)

**Problem:** Extension linked against `@rpath/libloaf.dylib` but dylib wasn't embedded.

**Solution:** Build script now:
1. Creates `LoafExtension.appex/Contents/Frameworks/`
2. Copies `libloaf.dylib` there
3. Signs dylib with Developer ID
4. Re-signs extension and app

### What We've Tried:
1. ✅ Signed with Developer ID Application certificate
2. ✅ Notarized with Apple
3. ✅ Hardened runtime enabled
4. ✅ Secure timestamp in signature
5. ✅ Added `com.apple.developer.fskit.fsmodule` entitlement to extension
6. ✅ Added `com.apple.security.cs.disable-library-validation` entitlement
7. ✅ Embedded and signed libloaf.dylib in extension's Frameworks folder
8. ✅ Manually enabled in root's enabledModules.plist
9. ✅ Restarted fskitd (`sudo pkill -HUP fskitd`)
10. ✅ Tested FSKitSample reference implementation (same failure)
11. ❌ Static linking (breaks notarization)

## Root Cause

FSKit on macOS 26 has multiple bugs preventing third-party extensions from working:

1. **Entitlement enforcement** - fskitd rejects connections from unprivileged clients
2. **Permission issues** - fskitd lacks permissions to access devices when mount is called
3. **KEXT priority** - System KEXTs are probed before FSKit modules, blocking third-party implementations

Apple is actively working on fixes. Per DTS engineer Kevin Elliott (July 2025):
> "Unfortunately, more bugs have been found so you're going to need to wait for more fixes."

Related Feedback reports:
- FB18230524 - System NTFS driver blocking FSKit modules
- FB17772372 - Probing issues (partially fixed in macOS 15.6 beta)

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

## Next Steps

1. **Wait for macOS fixes** - Apple is actively fixing FSKit bugs
2. **File Apple Feedback** - Report specific issues with error logs
3. **Monitor forums** - Watch [FSKit tag on Apple Developer Forums](https://developer.apple.com/forums/tags/fskit)
4. **Test on future macOS updates** - Check if 26.2+ or 27 fixes the issues

## References

- [FSKit module mount fails with permissions](https://developer.apple.com/forums/thread/788609) - Main issue thread
- [FSKit Sandbox restrictions](https://developer.apple.com/forums/thread/808246) - Sandbox workarounds
- [Cryptomator FSKit support issue](https://github.com/cryptomator/cryptomator/issues/3583) - Similar problems
- [macFUSE FSKit issue](https://github.com/macfuse/macfuse/issues/1025) - Framework status
- [FSKitSample](https://github.com/KhaosT/FSKitSample) - Reference implementation (deps/FSKitSample)
- [Apple FSKit Forums](https://developer.apple.com/forums/tags/fskit)
- [Eclectic Light FSKit Overview](https://eclecticlight.co/2024/06/26/how-file-systems-can-change-in-sequoia-with-fskit/)
