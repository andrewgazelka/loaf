import Foundation
import FSKit
import os

final class LoafItem: FSItem {

    private let logger = Logger(subsystem: "com.loaf", category: "LoafItem")
    private let fs: OpaquePointer
    let inodeId: UInt64

    var attributes: FSItem.Attributes

    init(fs: OpaquePointer, inodeId: UInt64) {
        self.fs = fs
        self.inodeId = inodeId
        self.attributes = FSItem.Attributes()
        super.init()

        refreshAttributes()
    }

    func refreshAttributes() {
        var attrs = loaf_attrs_t()
        let result = loaf_get_attrs(fs, inodeId, &attrs)
        guard result.rawValue == LOAF_OK.rawValue else {
            return
        }

        attributes.fileID = FSItem.Identifier(rawValue: inodeId) ?? .invalid
        attributes.parentID = FSItem.Identifier(rawValue: attrs.parent_id) ?? .parentOfRoot
        attributes.uid = attrs.uid
        attributes.gid = attrs.gid
        attributes.mode = UInt32(attrs.mode)
        attributes.linkCount = attrs.link_count
        attributes.size = attrs.size
        attributes.allocSize = attrs.alloc_size
        attributes.flags = attrs.flags
        attributes.accessTime = timespec(tv_sec: Int(attrs.atime_sec), tv_nsec: Int(attrs.atime_nsec))
        attributes.modifyTime = timespec(tv_sec: Int(attrs.mtime_sec), tv_nsec: Int(attrs.mtime_nsec))
        attributes.changeTime = timespec(tv_sec: Int(attrs.ctime_sec), tv_nsec: Int(attrs.ctime_nsec))
        attributes.birthTime = timespec(tv_sec: Int(attrs.btime_sec), tv_nsec: Int(attrs.btime_nsec))

        switch attrs.item_type {
        case 0: // file
            attributes.type = .file
        case 1: // directory
            attributes.type = .directory
        case 2: // symlink
            attributes.type = .symlink
        default:
            attributes.type = .file
        }
    }
}
