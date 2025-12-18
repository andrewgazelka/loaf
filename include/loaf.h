#ifndef LOAF_H
#define LOAF_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

// ============================================================================
// Types
// ============================================================================

/// Opaque filesystem handle.
typedef struct loaf_t loaf_t;

/// Opaque directory iterator.
typedef struct loaf_dir_iter_t loaf_dir_iter_t;

/// Error codes (POSIX errno values).
typedef enum {
    LOAF_OK = 0,
    LOAF_EPERM = 1,
    LOAF_ENOENT = 2,
    LOAF_EIO = 5,
    LOAF_ENOMEM = 12,
    LOAF_EACCES = 13,
    LOAF_EEXIST = 17,
    LOAF_ENOTDIR = 20,
    LOAF_EISDIR = 21,
    LOAF_EINVAL = 22,
    LOAF_ENOSPC = 28,
    LOAF_EROFS = 30,
    LOAF_ENAMETOOLONG = 63,
    LOAF_ENOTEMPTY = 66,
} loaf_error_t;

/// Item type.
typedef enum {
    LOAF_TYPE_FILE = 0,
    LOAF_TYPE_DIRECTORY = 1,
    LOAF_TYPE_SYMLINK = 2,
} loaf_item_type_t;

/// File/directory attributes.
typedef struct {
    uint64_t file_id;
    uint64_t parent_id;
    uint8_t item_type;
    uint8_t _pad1[3];
    uint32_t mode;
    uint32_t uid;
    uint32_t gid;
    uint64_t size;
    uint64_t alloc_size;
    uint32_t link_count;
    uint32_t flags;
    int64_t atime_sec;
    int64_t atime_nsec;
    int64_t mtime_sec;
    int64_t mtime_nsec;
    int64_t ctime_sec;
    int64_t ctime_nsec;
    int64_t btime_sec;
    int64_t btime_nsec;
} loaf_attrs_t;

// ============================================================================
// Lifecycle
// ============================================================================

/// Open a SQLite-backed filesystem (standalone mode, no overlay).
/// @param db_path Path to the SQLite database file.
/// @param out_fs Output pointer to the filesystem handle.
/// @return LOAF_OK on success, error code otherwise.
loaf_error_t loaf_open(const char* db_path, loaf_t** out_fs);

/// Open an overlay filesystem.
/// Reads check SQLite first, fall through to base_path.
/// Writes go to SQLite. Deletes create whiteouts.
/// @param db_path Path to the SQLite overlay database (.loaf file).
/// @param base_path Path to the real filesystem directory to overlay.
/// @param out_fs Output pointer to the filesystem handle.
/// @return LOAF_OK on success, error code otherwise.
loaf_error_t loaf_overlay_open(const char* db_path, const char* base_path, loaf_t** out_fs);

/// Open an overlay filesystem, auto-reading base_path from the database.
/// The .loaf file must have been created with `loaf init` which stores base_path.
/// @param db_path Path to the SQLite overlay database (.loaf file).
/// @param out_fs Output pointer to the filesystem handle.
/// @return LOAF_OK on success, LOAF_ENOENT if base_path not found in db.
loaf_error_t loaf_overlay_open_auto(const char* db_path, loaf_t** out_fs);

/// Close the filesystem and free all resources.
void loaf_close(loaf_t* fs);

// ============================================================================
// Navigation
// ============================================================================

/// Get the root inode ID (always 1).
uint64_t loaf_get_root_id(loaf_t* fs);

/// Get attributes for an inode.
loaf_error_t loaf_get_attrs(loaf_t* fs, uint64_t inode_id, loaf_attrs_t* out_attrs);

/// Look up a child by name in a directory.
loaf_error_t loaf_lookup(loaf_t* fs, uint64_t parent_id,
                         const char* name, size_t name_len,
                         uint64_t* out_inode_id);

// ============================================================================
// CRUD Operations
// ============================================================================

/// Create a file or directory.
loaf_error_t loaf_create(loaf_t* fs, uint64_t parent_id,
                         const char* name, size_t name_len,
                         loaf_item_type_t item_type, uint32_t mode,
                         uint64_t* out_inode_id);

/// Create a symbolic link.
loaf_error_t loaf_create_symlink(loaf_t* fs, uint64_t parent_id,
                                 const char* name, size_t name_len,
                                 const char* target, size_t target_len,
                                 uint64_t* out_inode_id);

/// Remove a file or empty directory.
loaf_error_t loaf_remove(loaf_t* fs, uint64_t parent_id, uint64_t inode_id);

/// Rename/move an item.
/// @param out_replaced_id If non-zero, the inode that was replaced at destination.
loaf_error_t loaf_rename(loaf_t* fs,
                         uint64_t src_parent_id, uint64_t src_inode_id,
                         uint64_t dst_parent_id,
                         const char* dst_name, size_t dst_name_len,
                         uint64_t* out_replaced_id);

// ============================================================================
// I/O Operations
// ============================================================================

/// Read from a file.
loaf_error_t loaf_read(loaf_t* fs, uint64_t inode_id, int64_t offset,
                       void* buf, size_t buf_len, size_t* out_bytes_read);

/// Write to a file.
loaf_error_t loaf_write(loaf_t* fs, uint64_t inode_id, int64_t offset,
                        const void* data, size_t data_len, size_t* out_bytes_written);

/// Read a symbolic link target.
loaf_error_t loaf_read_symlink(loaf_t* fs, uint64_t inode_id,
                               char* buf, size_t buf_len, size_t* out_len);

// ============================================================================
// Directory Enumeration
// ============================================================================

/// Begin iterating a directory.
loaf_error_t loaf_readdir_begin(loaf_t* fs, uint64_t dir_inode_id,
                                loaf_dir_iter_t** out_iter);

/// Get next directory entry.
/// @return LOAF_ENOENT when iteration is complete.
loaf_error_t loaf_readdir_next(loaf_dir_iter_t* iter,
                               uint64_t* out_inode_id,
                               const char** out_name,
                               size_t* out_name_len,
                               loaf_item_type_t* out_item_type);

/// End directory iteration and free resources.
void loaf_readdir_end(loaf_dir_iter_t* iter);

// ============================================================================
// Sync
// ============================================================================

/// Flush all pending writes to disk.
loaf_error_t loaf_sync(loaf_t* fs);

#ifdef __cplusplus
}
#endif

#endif // LOAF_H
