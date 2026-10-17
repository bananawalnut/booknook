// SPDX-License-Identifier: MPL-2.0
// Minimal domain projections for isolated renderer tests. Production projections
// remain in Models.swift and are verified by the full app build and SDK parity.
import Foundation
struct JSONValue { let object: [String: Any]? }
struct Annotation { let id: String; let inkGeometry: JSONValue?; let nativeDrawing: String?; let originalContext: String; let strokes: [JSONValue] }
