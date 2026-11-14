// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookServices)
import BookNookServices
#endif
private func rejected(_ operation: () throws -> Void) {
    var failure=false; do {try operation()} catch {failure=true}; precondition(failure)
}
private func pointer(_ object: Any,_ path: String) -> Any {
    path.split(separator:"/").reduce(object) {value,part in
        let key=part.replacingOccurrences(of:"~1",with:"/").replacingOccurrences(of:"~0",with:"~")
        if let array=value as? [Any],let index=Int(key) {return array[index]}
        return (value as! [String:Any])[key]!
    }
}
@main struct FeedXMLSDKBindingSmoke {
    static func main() throws {
        let corpus=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:CommandLine.arguments[1]))) as! [String:Any]
        let context=try JSONSerialization.data(withJSONObject:corpus["context"]!)
        for bad in [NSNull(),true,1,[Any](),["auto"],["auto":NSNull()]] as [Any] {
            var alias=corpus["context"] as! [String:Any];alias["expected_format"]=bad
            rejected {_ = try BookNookFeedXMLV1.normalize(context:JSONSerialization.data(withJSONObject:alias),xml:Data("<rss version='2.0'><channel/></rss>".utf8))}
        }
        let cases=corpus["cases"] as! [[String:Any]]
        for test in cases {
            let xml=Data((test["xml"] as! String).utf8)
            if !(test["valid"] as! Bool) {rejected {_ = try BookNookFeedXMLV1.normalize(context:context,xml:xml)};continue}
            let raw=try BookNookFeedXMLV1.normalize(context:context,xml:xml)
            let output=try JSONSerialization.jsonObject(with:Data(raw.utf8)) as! [String:Any]
            precondition(output["normalizer_profile"] as! String == "booknook.rss2-atom.inert.v1")
            for (path,expected) in test["checks"] as! [String:Any] {
                let actual=pointer(output,path)
                let actualJSON=try JSONSerialization.data(withJSONObject:[actual],options:[.sortedKeys])
                let expectedJSON=try JSONSerialization.data(withJSONObject:[expected],options:[.sortedKeys])
                precondition(actualJSON==expectedJSON,"\(test["id"]!) \(path)")
            }
        }
        rejected {_ = try BookNookFeedXMLV1.normalize(context:context,xml:Data([0xff,0xfe,60,0]))}
        for depth in [64,65] {
            let xml="<rss version='2.0'><channel>"+String(repeating:"<x>",count:depth-2)+String(repeating:"</x>",count:depth-2)+"</channel></rss>"
            if depth==64 {_ = try BookNookFeedXMLV1.normalize(context:context,xml:Data(xml.utf8))}
            else {rejected {_ = try BookNookFeedXMLV1.normalize(context:context,xml:Data(xml.utf8))}}
        }
        let deep="<rss version='2.0'><channel>"+String(repeating:"<x>",count:30_000)+String(repeating:"</x>",count:30_000)+"</channel></rss>"
        rejected {_ = try BookNookFeedXMLV1.normalize(context:context,xml:Data(deep.utf8))}
        print("Actual generated Swift XML normalizer: \(cases.count) shared malicious/synthetic cases, literal-string format refusals and hostile UTF8/depth passed; no transport/render/cache qualification.")
    }
}
