// SPDX-License-Identifier: MPL-2.0
import Foundation
import Darwin

/// Reads inert UTF-8 for the existing native binding validator; it does not
/// interpret JSON, install a key or change authority.
enum SocialBindingImport {
    static let maximumBytes = 300_000 // Existing limit is exclusive.

    static func readFile(_ url: URL) async throws -> String {
        try Task.checkCancellation()
        guard url.isFileURL, url.user == nil, url.password == nil,
              url.host == nil || url.host == "" || url.host?.lowercased() == "localhost",
              !url.path.utf8.contains(0) else {
            throw ConnectionInputError.message("Choose a local transport identity file.")
        }
        let reader = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            try Task.checkCancellation()
            // Validate the opened descriptor. O_NONBLOCK prevents waiting for a
            // FIFO writer even if the selected path is replaced or symlinked.
            // Regular/network-filesystem syscalls still have no deadline.
            let descriptor = try url.withUnsafeFileSystemRepresentation { path -> Int32 in
                guard let path else { throw ConnectionInputError.message("Choose a local transport identity file.") }
                let descriptor = Darwin.open(path, O_RDONLY | O_NONBLOCK | O_CLOEXEC)
                guard descriptor >= 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
                return descriptor
            }
            defer { _ = Darwin.close(descriptor) }
            var metadata = stat()
            guard Darwin.fstat(descriptor, &metadata) == 0 else { throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
            guard metadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG), metadata.st_size >= 0 else {
                throw ConnectionInputError.message("Choose a regular transport identity file.")
            }
            guard metadata.st_size < Int64(maximumBytes) else {
                throw ConnectionInputError.message("Identity file exceeds 300 KB")
            }
            var data = Data()
            var buffer = [UInt8](repeating: 0, count: 4_096)
            while true {
                try Task.checkCancellation()
                let count = buffer.withUnsafeMutableBytes { bytes in
                    Darwin.read(descriptor, bytes.baseAddress, min(bytes.count, maximumBytes - data.count))
                }
                if count < 0 {
                    if errno == EINTR { continue }
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
                }
                if count == 0 { break }
                data.append(contentsOf: buffer.prefix(count))
                guard data.count < maximumBytes else { throw ConnectionInputError.message("Identity file exceeds 300 KB") }
            }
            try Task.checkCancellation()
            guard let text = String(data: data, encoding: .utf8) else {
                throw ConnectionInputError.message("Choose a UTF-8 transport identity file.")
            }
            return text
        }
        return try await withTaskCancellationHandler {
            let text = try await reader.value
            try Task.checkCancellation()
            return text
        } onCancel: { reader.cancel() }
    }
}
