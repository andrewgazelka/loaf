import Foundation
import FSKit
import os

final class LoafFileSystem: FSUnaryFileSystem, FSUnaryFileSystemOperations {

    private let logger = Logger(subsystem: "com.loaf", category: "LoafFileSystem")

    func probeResource(
        resource: FSResource,
        replyHandler: @escaping (FSProbeResult?, (any Error)?) -> Void
    ) {
        logger.debug("probeResource: \(resource, privacy: .public)")

        // Check if this is a .loaf file
        guard let pathResource = resource as? FSPathURLResource else {
            logger.error("Resource is not a path URL resource")
            replyHandler(nil, fs_errorForPOSIXError(POSIXError.EINVAL.rawValue))
            return
        }

        let path = pathResource.url.path
        guard path.hasSuffix(".loaf") else {
            logger.error("Resource is not a .loaf file: \(path, privacy: .public)")
            replyHandler(nil, fs_errorForPOSIXError(POSIXError.EINVAL.rawValue))
            return
        }

        replyHandler(
            FSProbeResult.usable(
                name: "Loaf",
                containerID: FSContainerIdentifier(uuid: UUID())
            ),
            nil
        )
    }

    func loadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler: @escaping (FSVolume?, (any Error)?) -> Void
    ) {
        logger.debug("loadResource: \(resource, privacy: .public)")

        // Extract path from FSPathURLResource
        guard let pathResource = resource as? FSPathURLResource else {
            logger.error("Resource is not a path URL resource")
            replyHandler(nil, fs_errorForPOSIXError(POSIXError.EINVAL.rawValue))
            return
        }

        let path = pathResource.url.path
        logger.info("Loading .loaf file: \(path, privacy: .public)")

        do {
            let volume = try LoafVolume(path: path)
            containerStatus = .ready
            replyHandler(volume, nil)
        } catch {
            logger.error("Failed to load resource: \(error, privacy: .public)")
            replyHandler(nil, error)
        }
    }

    func unloadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping ((any Error)?) -> Void
    ) {
        logger.debug("unloadResource: \(resource, privacy: .public)")
        reply(nil)
    }

    func didFinishLoading() {
        logger.debug("didFinishLoading")
    }
}
