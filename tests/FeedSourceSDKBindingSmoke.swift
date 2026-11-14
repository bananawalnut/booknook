// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import CryptoKit
private func object(_ raw: String) throws -> [String:Any] {try JSONSerialization.jsonObject(with:Data(raw.utf8)) as! [String:Any]}
private func bytes(_ value: Any) throws -> Data {try JSONSerialization.data(withJSONObject:value,options:[.sortedKeys])}
private func rejected(_ operation: () throws -> Void) {var failed=false;do {try operation()}catch{failed=true};precondition(failed)}
private func rejectedAsync(_ operation: () async throws -> Void) async {var failed=false;do{try await operation()}catch{failed=true};precondition(failed)}
private final class Count: @unchecked Sendable {private let lock=NSLock();private var value=0;func increment(){lock.lock();value+=1;lock.unlock()};func get()->Int{lock.lock();defer{lock.unlock()};return value}}
private struct Source: BookNookFeedSourceProviderV1 {
    let before: @Sendable () async throws -> Void
    let waitForCancel: Bool
    let calls: Count
    var corrupt: String? = nil
    var descriptorJSON:String {"{\"schema\":\"booknook.feed-source-provider.v1\",\"version\":1,\"id\":\"reader.public\",\"available\":true,\"protocols\":[\"public-xml-bytes.v1\"]}"}
    func fetch(requestJSON:String,cancellation:FeedSourceCancellationV1) async throws -> BookNookFeedSourceReplyV1 {
        calls.increment();let request=try object(requestJSON);precondition(request.count==12);precondition(request["title"]==nil && request["collections"]==nil && request["identity"]==nil)
        await Task.yield();try await before()
        if waitForCancel {while !cancellation.isCancelled(){await Task.yield()};throw CancellationError()}
        let conditional=request["conditional"] as? [String:Any];let body=conditional == nil ? Data("<rss/>".utf8) : Data();var response=request
        response["schema"]="booknook.feed-source-response.v1";response.removeValue(forKey:"url");response.removeValue(forKey:"format");response.removeValue(forKey:"protocol");response.removeValue(forKey:"conditional");response["outcome"]=conditional == nil ? "body" : "not-modified";response["validators"]=["etag":NSNull(),"last_modified":NSNull()]
        response["final_url"]=request["url"];response["content_type"]=conditional == nil ? "application/rss+xml" as Any : NSNull();response["body_sha256"]=conditional?["body_sha256"] ?? SHA256.hash(data:body).map{String(format:"%02x",$0)}.joined();if let field=corrupt {response[field]=field.hasSuffix("generation") ? 999 : (field == "final_url" ? "https://other.example.org/feed" : "wrong") as Any}
        return BookNookFeedSourceReplyV1(responseBytes:try bytes(response),untrustedBody:body)
    }
}
private struct Cache: BookNookFeedCacheStoreV1 {
    let runtime: BookNookFeedSourcesV1
    let writes:Count
    let before: @Sendable () async throws -> Void
    var retained: Data? = nil
    func stage(conditionJSON:String,untrustedBody:Data,cancellation:FeedSourceCancellationV1) async throws -> BookNookFeedCacheStageV1 {let condition=try object(conditionJSON);precondition(condition["schema"] as! String == "booknook.feed-cache-condition.v1");await Task.yield();try await before();return BookNookFeedCacheStageV1(reference:"staged",existingBody:retained)}
    func compareAndStore(staged:String,conditionJSON:String,untrustedBody:Data) throws {precondition(staged=="staged" && (retained == nil ? untrustedBody==Data("<rss/>".utf8) : untrustedBody.isEmpty));let source=try object(conditionJSON)["source_id"] as! String;rejected{try runtime.disable(sourceID:source)};writes.increment()}
}
@main struct FeedSourceSDKBindingSmoke {
    static func main() async throws {
        let root=URL(fileURLWithPath:CommandLine.arguments[1]);let feeds=try JSONSerialization.jsonObject(with:Data(contentsOf:root.appendingPathComponent("section.json"))) as! [String:Any]
        let config=try bytes(["schema":"booknook.feed-configuration.v1","revision":0,"feeds":feeds]);let id=(feeds["subscriptions"] as! [[String:Any]])[0]["id"] as! String
        let calls=Count();let provider=Source(before:{},waitForCancel:false,calls:calls);let runtime=try BookNookFeedSourcesV1(configuration:config,provider:provider)
        await rejectedAsync{_ = try await runtime.fetch(sourceID:id)};precondition(calls.get()==0);try runtime.activate(sourceID:id)
        let read=try await runtime.fetch(sourceID:id);precondition(read.untrustedBody()==Data("<rss/>".utf8));let writes=Count()
        try await read.store(Cache(runtime:runtime,writes:writes,before:{}));precondition(writes.get()==1);rejected{_ = try read.cacheConditionJSON()}
        try runtime.activate(sourceID:id);let old=try await runtime.fetch(sourceID:id);let current=try await runtime.fetch(sourceID:id);rejected{_ = try old.cacheConditionJSON()};old.close();current.close()
        let stale=try await runtime.fetch(sourceID:id)
        await rejectedAsync{try await stale.store(Cache(runtime:runtime,writes:writes,before:{try runtime.replaceConfiguration(config)}))};precondition(writes.get()==1)
        try runtime.replaceProvider(Source(before:{try runtime.replaceConfiguration(config)},waitForCancel:false,calls:calls));try runtime.activate(sourceID:id)
        await rejectedAsync{_ = try await runtime.fetch(sourceID:id)}
        let cancelCalls=Count();try runtime.replaceProvider(Source(before:{},waitForCancel:true,calls:cancelCalls));try runtime.activate(sourceID:id)
        let task=Task{try await runtime.fetch(sourceID:id)}
        let startDeadline=Date().addingTimeInterval(5);while cancelCalls.get()==0 && Date()<startDeadline {try await Task.sleep(nanoseconds:1_000_000)};precondition(cancelCalls.get()>0);task.cancel();await rejectedAsync{_ = try await task.value}
        try runtime.replaceProvider(provider);try runtime.activate(sourceID:id)
        let digest=SHA256.hash(data:Data("<rss/>".utf8)).map{String(format:"%02x",$0)}.joined()
        let hint=try bytes(["schema":"booknook.feed-conditional-hint.v1","source_id":id,"provider_id":"reader.public","url":(feeds["subscriptions"] as! [[String:Any]])[0]["url"]!,"body_sha256":digest,"validators":["etag":"\"v1\"","last_modified":NSNull()]] as [String:Any])
        for retained in [nil,Data("wrong".utf8),Data("<rss/>".utf8)] as [Data?] {let read=try await runtime.fetch(sourceID:id,conditional:hint);precondition(read.untrustedBody().isEmpty);let before=writes.get();let cache=Cache(runtime:runtime,writes:writes,before:{},retained:retained);if retained==Data("<rss/>".utf8){try await read.store(cache);precondition(writes.get()==before+1)}else{await rejectedAsync{try await read.store(cache)};precondition(writes.get()==before)};read.close()}
        for field in ["provider_id","configuration_generation","final_url","body_sha256"] {try runtime.replaceProvider(Source(before:{},waitForCancel:false,calls:calls,corrupt:field));try runtime.activate(sourceID:id);await rejectedAsync{_ = try await runtime.fetch(sourceID:id,conditional:hint)}}
        try runtime.replaceProvider(provider);try runtime.activate(sourceID:id);let closing=try await runtime.fetch(sourceID:id);try runtime.close();rejected{_ = try closing.cacheConditionJSON()};await rejectedAsync{_ = try await runtime.fetch(sourceID:id)}
        print("Actual generated Swift async source SDK: explicit activation, cancellation, stale provider/config/storage rejection and guarded foreign cache reentry passed. No transport/backend qualification claimed.")
    }
}
