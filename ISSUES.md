# FSKit Issues on macOS 26

## Current Status

**FSKit third-party extensions do not work on macOS 26.** This is an Apple bug, not a Loaf issue.

| macOS Version | Build | Status |
|--------------|-------|--------|
| 26.2 | 25C56 | ❌ Broken |
| 26.1 | 25B78 | ❌ Broken |

## Architecture Overview

```mermaid
flowchart TB
    subgraph UserSpace["User Space"]
        mount["mount -t loaf"]
        app["Loaf.app"]
        ext["LoafExtension.appex"]
    end

    subgraph System["System Daemons"]
        fskitd["fskitd"]
        lsd["lsd (Launch Services)"]
        fskit_agent["fskit_agent"]
    end

    subgraph Kernel["Kernel"]
        vfs["VFS Layer"]
    end

    mount -->|"XPC connection"| fskitd
    fskitd -->|"entitlement check"| fskitd
    fskitd -.->|"❌ FAILS HERE"| ext
    lsd -->|"registers"| ext
    fskitd -->|"spawns"| fskit_agent
    fskit_agent -->|"loads modules"| ext
    ext -->|"filesystem ops"| vfs
```

## The Failure Point

```mermaid
sequenceDiagram
    participant User
    participant mount
    participant fskitd
    participant Extension

    User->>mount: mount -t loaf /path
    mount->>fskitd: XPC connection request

    Note over fskitd: Checks entitlements
    fskitd-->>fskitd: entitled = 0 ❌

    fskitd->>mount: "Hello FSClient! entitlement no"

    Note over fskitd: Attempts to start extension anyway
    fskitd->>Extension: Start instance
    Extension-->>fskitd: RBSRequestErrorDomain Code=5

    fskitd->>mount: extensionKit.errorDomain Code=2
    mount->>User: "Probing resource failed"
```

## Error Chain

```mermaid
flowchart LR
    subgraph Errors["Error Propagation"]
        rbs["RBSRequestErrorDomain<br/>Code=5"]
        ext1["extensionKit.errorDomain<br/>Code=2"]
        ext2["extensionKit.errorDomain<br/>Code=4"]
        ext3["extensionKit.errorDomain<br/>Code=2"]
        final["mount: Unable to<br/>invoke task"]
    end

    rbs --> ext1 --> ext2 --> ext3 --> final

    style rbs fill:#f66,stroke:#333
    style final fill:#f66,stroke:#333
```

## What's Broken

```mermaid
mindmap
  root((FSKit on<br/>macOS 26))
    Entitlement Enforcement
      fskitd rejects unprivileged clients
      "entitled 0" logged for all third-party
      Only Apple extensions pass
    Extension Launch
      RBSRequestErrorDomain Code=5
      RunningBoard refuses to start
      Extension never executes
    Toggle Bounce
      System Settings toggle reverts
      Workaround: edit plist manually
      Both user and root plists needed
    KEXT Priority
      System KEXTs probed first
      Third-party FSKit deprioritized
      DiskArbitration blocking
```

## Components Status

```mermaid
flowchart TB
    subgraph Working["✅ Working"]
        zig["Zig Core Library"]
        sqlite["SQLite Operations"]
        swift["Swift FSKit Extension"]
        notarize["Notarization"]
        sign["Code Signing"]
        register["Extension Registration"]
    end

    subgraph Broken["❌ Broken (Apple Bug)"]
        entitle["Entitlement Check"]
        launch["Extension Launch"]
        mount_op["Mount Operation"]
    end

    zig --> swift
    swift --> sign
    sign --> notarize
    notarize --> register
    register --> entitle
    entitle -.->|"BLOCKED"| launch
    launch -.->|"BLOCKED"| mount_op

    style entitle fill:#f66
    style launch fill:#f66
    style mount_op fill:#f66
```

## Log Evidence

```
fskitd: [com.apple.FSKit:default] Incomming connection, entitled 0
fskitd: [com.apple.FSKit:default] Hello FSClient! entitlement no
fskitd: [com.apple.FSKit:default] About to get current agent for 501
fskitd: [com.apple.FSKit:default] Failed to start instance <private>:
  Error Domain=com.apple.extensionKit.errorDomain Code=2
  UserInfo={NSUnderlyingError=... RBSRequestErrorDomain Code=5 ...}
```

## Apple Feedback Reports

| Feedback ID | Issue | Status |
|------------|-------|--------|
| FB18230524 | System NTFS driver blocking FSKit modules | Open |
| FB17772372 | Probing issues | Partially fixed in 15.6 beta |

## References

- [FSKit module mount fails with permissions](https://developer.apple.com/forums/thread/788609)
- [FSKit Sandbox restrictions](https://developer.apple.com/forums/thread/808246)
- [Cryptomator FSKit support issue](https://github.com/cryptomator/cryptomator/issues/3583)
- [macFUSE FSKit issue](https://github.com/macfuse/macfuse/issues/1025)

## Workarounds Attempted

All of these do NOT fix the issue:

1. ✅ Developer ID signing
2. ✅ Apple notarization
3. ✅ Hardened runtime
4. ✅ FSKit entitlements
5. ✅ Library validation disabled
6. ✅ Embedded dylib in extension
7. ✅ Manual plist enablement
8. ✅ fskitd restart
9. ❌ Static linking (breaks notarization)

## Conclusion

We are blocked waiting for Apple to fix FSKit. Per DTS engineer Kevin Elliott (July 2025):

> "Unfortunately, more bugs have been found so you're going to need to wait for more fixes."
