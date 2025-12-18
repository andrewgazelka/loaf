/// POSIX-compatible error codes for FFI.
pub const FfiError = enum(c_int) {
    ok = 0,
    eperm = 1,
    enoent = 2,
    eio = 5,
    enomem = 12,
    eacces = 13,
    eexist = 17,
    enotdir = 20,
    eisdir = 21,
    einval = 22,
    enospc = 28,
    erofs = 30,
    enametoolong = 63,
    enotempty = 66,
};

/// Internal error set for Zig code.
pub const Error = error{
    NotFound,
    PermissionDenied,
    IoError,
    OutOfMemory,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    InvalidArgument,
    NoSpace,
    ReadOnly,
    NameTooLong,
    NotEmpty,
    NotASymlink,
    InvalidOffset,
    QueryFailed,
    InsertFailed,
    UpdateFailed,
    DeleteFailed,
    SyncFailed,
    SchemaInitFailed,
    RootInsertFailed,
};

/// Convert internal error to FFI error code.
pub fn toFfi(err: Error) FfiError {
    return switch (err) {
        error.NotFound => .enoent,
        error.PermissionDenied => .eacces,
        error.IoError, error.QueryFailed, error.InsertFailed, error.UpdateFailed, error.DeleteFailed, error.SyncFailed, error.SchemaInitFailed, error.RootInsertFailed => .eio,
        error.OutOfMemory => .enomem,
        error.AlreadyExists => .eexist,
        error.NotDirectory => .enotdir,
        error.IsDirectory => .eisdir,
        error.InvalidArgument, error.NotASymlink, error.InvalidOffset => .einval,
        error.NoSpace => .enospc,
        error.ReadOnly => .erofs,
        error.NameTooLong => .enametoolong,
        error.NotEmpty => .enotempty,
    };
}
