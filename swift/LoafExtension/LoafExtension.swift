import Foundation
import FSKit

@main
struct LoafExtension: UnaryFileSystemExtension {
    var fileSystem: FSUnaryFileSystem & FSUnaryFileSystemOperations {
        LoafFileSystem()
    }
}
