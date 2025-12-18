import Foundation
import FSKit
import os

final class LoafVolume: FSVolume {

    private let logger = Logger(subsystem: "com.loaf", category: "LoafVolume")
    private var fs: OpaquePointer?
    private var rootItem: LoafItem!

    init(path: String) throws {
        var fsPtr: OpaquePointer?
        // Use overlay mode - reads from real FS, writes to SQLite
        let result = loaf_overlay_open_auto(path, &fsPtr)
        guard result.rawValue == LOAF_OK.rawValue, let fs = fsPtr else {
            // If overlay fails (no base_path stored), fall back to standalone mode
            let standaloneResult = loaf_open(path, &fsPtr)
            guard standaloneResult.rawValue == LOAF_OK.rawValue, let standalonefs = fsPtr else {
                throw NSError(domain: "LoafError", code: Int(standaloneResult.rawValue))
            }
            self.fs = standalonefs

            super.init(
                volumeID: FSVolume.Identifier(uuid: UUID()),
                volumeName: FSFileName(string: URL(fileURLWithPath: path).lastPathComponent)
            )

            self.rootItem = LoafItem(fs: standalonefs, inodeId: loaf_get_root_id(standalonefs))
            return
        }
        self.fs = fs

        super.init(
            volumeID: FSVolume.Identifier(uuid: UUID()),
            volumeName: FSFileName(string: URL(fileURLWithPath: path).lastPathComponent)
        )

        self.rootItem = LoafItem(fs: fs, inodeId: loaf_get_root_id(fs))
    }

    deinit {
        if let fs = fs {
            loaf_close(fs)
        }
    }
}

// MARK: - FSVolume.PathConfOperations

extension LoafVolume: FSVolume.PathConfOperations {

    var maximumLinkCount: Int { 32000 }
    var maximumNameLength: Int { 255 }
    var restrictsOwnershipChanges: Bool { false }
    var truncatesLongNames: Bool { false }
    var maximumXattrSize: Int { Int.max }
    var maximumFileSize: UInt64 { UInt64.max }
}

// MARK: - FSVolume.Operations

extension LoafVolume: FSVolume.Operations {

    var supportedVolumeCapabilities: FSVolume.SupportedCapabilities {
        let caps = FSVolume.SupportedCapabilities()
        caps.supportsSymbolicLinks = true
        caps.supportsPersistentObjectIDs = true
        caps.supports64BitObjectIDs = true
        caps.caseFormat = .insensitiveCasePreserving
        return caps
    }

    var volumeStatistics: FSStatFSResult {
        let result = FSStatFSResult(fileSystemTypeName: "loaf")
        result.blockSize = 4096
        result.ioSize = 4096
        result.totalBlocks = 1_000_000
        result.availableBlocks = 500_000
        result.freeBlocks = 500_000
        result.totalFiles = 100_000
        result.freeFiles = 50_000
        return result
    }

    func activate(options: FSTaskOptions) async throws -> FSItem {
        logger.debug("activate")
        return rootItem
    }

    func deactivate(options: FSDeactivateOptions = []) async throws {
        logger.debug("deactivate")
        if let fs = fs {
            _ = loaf_sync(fs)
        }
    }

    func mount(options: FSTaskOptions) async throws {
        logger.debug("mount")
    }

    func unmount() async {
        logger.debug("unmount")
        if let fs = fs {
            _ = loaf_sync(fs)
        }
    }

    func synchronize(flags: FSSyncFlags) async throws {
        logger.debug("synchronize")
        if let fs = fs {
            _ = loaf_sync(fs)
        }
    }

    func attributes(
        _ desiredAttributes: FSItem.GetAttributesRequest,
        of item: FSItem
    ) async throws -> FSItem.Attributes {
        guard let item = item as? LoafItem else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }
        return item.attributes
    }

    func setAttributes(
        _ newAttributes: FSItem.SetAttributesRequest,
        on item: FSItem
    ) async throws -> FSItem.Attributes {
        guard let item = item as? LoafItem else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }
        // TODO: Call loaf_set_attrs when implemented
        return item.attributes
    }

    func lookupItem(
        named name: FSFileName,
        inDirectory directory: FSItem
    ) async throws -> (FSItem, FSFileName) {
        guard let dir = directory as? LoafItem, let fs = fs, let nameStr = name.string else {
            throw fs_errorForPOSIXError(POSIXError.ENOENT.rawValue)
        }

        var inodeId: UInt64 = 0
        let result = nameStr.withCString { cstr in
            loaf_lookup(fs, dir.inodeId, cstr, strlen(cstr), &inodeId)
        }

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.ENOENT.rawValue)
        }

        return (LoafItem(fs: fs, inodeId: inodeId), name)
    }

    func reclaimItem(_ item: FSItem) async throws {
        logger.debug("reclaimItem")
    }

    func readSymbolicLink(_ item: FSItem) async throws -> FSFileName {
        guard let item = item as? LoafItem, let fs = fs else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        var buf = [CChar](repeating: 0, count: 4096)
        var len: Int = 0
        let result = loaf_read_symlink(fs, item.inodeId, &buf, buf.count, &len)

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        let target = String(cString: buf)
        return FSFileName(string: target)
    }

    func createItem(
        named name: FSFileName,
        type: FSItem.ItemType,
        inDirectory directory: FSItem,
        attributes newAttributes: FSItem.SetAttributesRequest
    ) async throws -> (FSItem, FSFileName) {
        guard let dir = directory as? LoafItem, let fs = fs, let nameStr = name.string else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        let itemType: loaf_item_type_t = switch type {
        case .file: LOAF_TYPE_FILE
        case .directory: LOAF_TYPE_DIRECTORY
        case .symlink: LOAF_TYPE_SYMLINK
        default: LOAF_TYPE_FILE
        }

        var inodeId: UInt64 = 0
        let result = nameStr.withCString { cstr in
            loaf_create(fs, dir.inodeId, cstr, strlen(cstr), itemType, newAttributes.mode, &inodeId)
        }

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        return (LoafItem(fs: fs, inodeId: inodeId), name)
    }

    func createSymbolicLink(
        named name: FSFileName,
        inDirectory directory: FSItem,
        attributes newAttributes: FSItem.SetAttributesRequest,
        linkContents contents: FSFileName
    ) async throws -> (FSItem, FSFileName) {
        guard let dir = directory as? LoafItem, let fs = fs,
              let nameStr = name.string, let targetStr = contents.string else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        var inodeId: UInt64 = 0
        let result = nameStr.withCString { nameCstr in
            targetStr.withCString { targetCstr in
                loaf_create_symlink(fs, dir.inodeId, nameCstr, strlen(nameCstr),
                                    targetCstr, strlen(targetCstr), &inodeId)
            }
        }

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        return (LoafItem(fs: fs, inodeId: inodeId), name)
    }

    func createLink(
        to item: FSItem,
        named name: FSFileName,
        inDirectory directory: FSItem
    ) async throws -> FSFileName {
        // Hard links not yet implemented in Zig core
        throw fs_errorForPOSIXError(POSIXError.ENOTSUP.rawValue)
    }

    func removeItem(
        _ item: FSItem,
        named name: FSFileName,
        fromDirectory directory: FSItem
    ) async throws {
        guard let item = item as? LoafItem, let dir = directory as? LoafItem, let fs = fs else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        let result = loaf_remove(fs, dir.inodeId, item.inodeId)
        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }
    }

    func renameItem(
        _ item: FSItem,
        inDirectory sourceDirectory: FSItem,
        named sourceName: FSFileName,
        to destinationName: FSFileName,
        inDirectory destinationDirectory: FSItem,
        overItem: FSItem?
    ) async throws -> FSFileName {
        guard let item = item as? LoafItem,
              let srcDir = sourceDirectory as? LoafItem,
              let dstDir = destinationDirectory as? LoafItem,
              let fs = fs,
              let dstName = destinationName.string else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        var replacedId: UInt64 = 0
        let result = dstName.withCString { cstr in
            loaf_rename(fs, srcDir.inodeId, item.inodeId, dstDir.inodeId,
                        cstr, strlen(cstr), &replacedId)
        }

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        return destinationName
    }

    func enumerateDirectory(
        _ directory: FSItem,
        startingAt cookie: FSDirectoryCookie,
        verifier: FSDirectoryVerifier,
        attributes: FSItem.GetAttributesRequest?,
        packer: FSDirectoryEntryPacker
    ) async throws -> FSDirectoryVerifier {
        guard let dir = directory as? LoafItem, let fs = fs else {
            throw fs_errorForPOSIXError(POSIXError.ENOENT.rawValue)
        }

        var iterPtr: OpaquePointer?
        var result = loaf_readdir_begin(fs, dir.inodeId, &iterPtr)
        guard result.rawValue == LOAF_OK.rawValue, let iter = iterPtr else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }
        defer { loaf_readdir_end(iter) }

        var idx: UInt64 = 0
        while true {
            var inodeId: UInt64 = 0
            var namePtr: UnsafePointer<CChar>?
            var nameLen: Int = 0
            var itemType: loaf_item_type_t = LOAF_TYPE_FILE

            result = loaf_readdir_next(iter, &inodeId, &namePtr, &nameLen, &itemType)
            if result.rawValue != LOAF_OK.rawValue { break }

            guard let namePtr = namePtr else { continue }
            let name = String(cString: namePtr)
            let fsName = FSFileName(string: name)

            let fsType: FSItem.ItemType = switch itemType {
            case LOAF_TYPE_FILE: .file
            case LOAF_TYPE_DIRECTORY: .directory
            case LOAF_TYPE_SYMLINK: .symlink
            default: .file
            }

            let childItem = LoafItem(fs: fs, inodeId: inodeId)
            if let itemID = FSItem.Identifier(rawValue: inodeId) {
                _ = packer.packEntry(
                    name: fsName,
                    itemType: fsType,
                    itemID: itemID,
                    nextCookie: FSDirectoryCookie(idx),
                    attributes: attributes != nil ? childItem.attributes : nil
                )
            }
            idx += 1
        }

        return FSDirectoryVerifier(0)
    }
}

// MARK: - FSVolume.OpenCloseOperations

extension LoafVolume: FSVolume.OpenCloseOperations {

    func openItem(_ item: FSItem, modes: FSVolume.OpenModes) async throws {
        logger.debug("open")
    }

    func closeItem(_ item: FSItem, modes: FSVolume.OpenModes) async throws {
        logger.debug("close")
    }
}

// MARK: - FSVolume.ReadWriteOperations

extension LoafVolume: FSVolume.ReadWriteOperations {

    func read(from item: FSItem, at offset: off_t, length: Int, into buffer: FSMutableFileDataBuffer) async throws -> Int {
        guard let item = item as? LoafItem, let fs = fs else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        var bytesRead: Int = 0
        let result = buffer.withUnsafeMutableBytes { ptr in
            loaf_read(fs, item.inodeId, Int64(offset), ptr.baseAddress, length, &bytesRead)
        }

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        return bytesRead
    }

    func write(contents: Data, to item: FSItem, at offset: off_t) async throws -> Int {
        guard let item = item as? LoafItem, let fs = fs else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        var bytesWritten: Int = 0
        let result = contents.withUnsafeBytes { ptr in
            loaf_write(fs, item.inodeId, Int64(offset), ptr.baseAddress, contents.count, &bytesWritten)
        }

        guard result.rawValue == LOAF_OK.rawValue else {
            throw fs_errorForPOSIXError(POSIXError.EIO.rawValue)
        }

        return bytesWritten
    }
}
