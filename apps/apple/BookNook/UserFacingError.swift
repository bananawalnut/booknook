// SPDX-License-Identifier: MPL-2.0
import Foundation

/// The generated UniFFI error description includes Swift enum syntax. Show its
/// actionable message instead, while preserving the underlying failure for callers.
func bookNookErrorMessage(_ error: Error) -> String {
    if let native = error as? NativeError, case .Failure(let message) = native {
        return message
    }
    return error.localizedDescription
}

func bookNookOperationWasCancelled(_ error: Error) -> Bool {
    if error is CancellationError { return true }
    let cocoa = error as NSError
    return cocoa.domain == NSCocoaErrorDomain && cocoa.code == NSUserCancelledError
}
