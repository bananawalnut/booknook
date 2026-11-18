// SPDX-License-Identifier: MPL-2.0
import Darwin
import Foundation

/// Read the same regular-file descriptor we validated, while the native grant is held.
func readSelectedRegularFile(
    _ url: URL, maximumBytes: Int, priority: TaskPriority = .userInitiated,
    invalidFile: Error, exceedsLimit: Error
) async throws -> Data {
    precondition(maximumBytes >= 0 && maximumBytes < Int.max)
    try Task.checkCancellation()
    guard url.isFileURL, !url.path.utf8.contains(0) else { throw invalidFile }
    let reader = Task.detached(priority: priority) {
        try Task.checkCancellation()
        let accessing = url.startAccessingSecurityScopedResource()
        defer { if accessing { url.stopAccessingSecurityScopedResource() } }
        try Task.checkCancellation()

        // Nonblocking open avoids waiting on a nonregular file. Preserve the
        // picker's regular-file boundary by rejecting a final symbolic link.
        let descriptor = try url.withUnsafeFileSystemRepresentation { path in
            guard let path else { throw invalidFile }
            let descriptor = Darwin.open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW)
            if descriptor >= 0 { return descriptor }
            let code = errno
            if code == ELOOP { throw invalidFile }
            throw selectedFileReadError(code, url)
        }
        defer { _ = Darwin.close(descriptor) }

        var information = stat()
        guard Darwin.fstat(descriptor, &information) == 0 else { throw selectedFileReadError(errno, url) }
        guard information.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG),
              information.st_size >= 0, information.st_size <= Int64(maximumBytes) else { throw invalidFile }

        var bytes = Data()
        var buffer = [UInt8](repeating: 0, count: min(1_048_576, maximumBytes + 1))
        while true {
            try Task.checkCancellation()
            // Check EOF after the last permitted byte. If the file grew since
            // fstat, refuse it before adding the excess bytes to the result.
            let count = buffer.withUnsafeMutableBytes { storage in
                Darwin.read(descriptor, storage.baseAddress, min(storage.count, maximumBytes + 1 - bytes.count))
            }
            if count < 0 {
                let code = errno
                if code == EINTR { continue }
                throw selectedFileReadError(code, url)
            }
            if count == 0 { return bytes }
            guard count <= maximumBytes - bytes.count else { throw exceedsLimit }
            bytes.append(contentsOf: buffer.prefix(count))
        }
    }
    return try await withTaskCancellationHandler {
        let bytes = try await reader.value
        try Task.checkCancellation()
        return bytes
    } onCancel: {
        reader.cancel()
    }
}

private func selectedFileReadError(_ code: Int32, _ url: URL) -> CocoaError {
    let kind: CocoaError.Code
    switch code {
    case ENOENT, ENOTDIR: kind = .fileReadNoSuchFile
    case EACCES, EPERM: kind = .fileReadNoPermission
    default: kind = .fileReadUnknown
    }
    return CocoaError(kind, userInfo: [NSURLErrorKey: url,
        NSUnderlyingErrorKey: NSError(domain: NSPOSIXErrorDomain, code: Int(code))])
}
