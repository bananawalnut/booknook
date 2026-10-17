// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers

private enum SocialFileImport {
    case identity, history
}

struct SocialCommunityView: View {
    @EnvironmentObject var model: DeskModel
    @StateObject private var social = SocialController()
    @State private var tab = "Feed"
    @State private var selectedBook = ""
    @State private var kind = "note"
    @State private var bodyText = ""
    @State private var stars = 0
    @State private var composerExpanded = false
    @State private var readingStatus = "currently-reading"
    @State private var spoiler = false
    @State private var peer = ""
    @State private var name = ""
    @State private var description = ""
    @State private var privateGroup = true
    @State private var selectedGroup = ""
    @State private var selectedList = ""
    @State private var selectedBooks = Set<String>()
    @State private var invitee = ""
    @State private var interests = ""
    @State private var bindingText = ""
    @State private var importing = false
    @State private var fileImport: SocialFileImport = .identity
    @State private var exporting = false
    @State private var exportDocument = SocialFileDocument(text: "")
    @State private var exportName = "booknook-transport-identity.json"
    private var state: SocialStateModel? { social.snapshot?.state }
    private var group: SocialGroup? { state?.groups[selectedGroup] }
    private var groups: [SocialGroup] { (state?.groups.values.map { $0 } ?? []).filter { !$0.closed }.sorted { $0.name < $1.name } }
    private var publicGroups: [SocialGroup] { groups.filter { !$0.private && state?.blocked.contains($0.owner) != true } }
    private var sharedLists: [SocialList] { (state?.lists.values.map { $0 } ?? []).filter { !$0.deleted }.sorted { $0.name < $1.name } }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 8) { Text(tab == "Discover" ? "Find your next read" : "A shared love of reading").font(.system(size: 32, design: .serif)); Text(tab == "Discover" ? "Follow your interests, at your own pace." : "A chronological conversation, at your own pace.").foregroundStyle(.secondary) }
                    Spacer()
                    if social.busy { ProgressView().controlSize(.small) }
                }
                Picker("Community", selection: $tab) { ForEach(["Feed", "Friends", "Lists", "Groups", "Discover", "Queue", "Transport"], id: \.self) { Text($0 == "Queue" ? "Outbox" : $0 == "Transport" ? "Sharing" : $0).tag($0) } }.pickerStyle(.segmented)
                if social.actor.isEmpty && tab != "Discover" && tab != "Transport" { Label("Set up sharing to publish, make friends, or join a group. Your library and book suggestions work offline.", systemImage: "person.crop.circle.badge.exclamationmark").foregroundStyle(.secondary); Button("Set up sharing") { tab = "Transport" } }
                if !social.status.isEmpty { Text(social.status).font(.callout).foregroundStyle(.secondary).textSelection(.enabled) }
                if let error = social.error, tab != "Discover" { HStack(alignment: .top) { Image(systemName: "exclamationmark.circle"); Text(error).textSelection(.enabled); Spacer(); Button("Dismiss") { social.error = nil } }.foregroundStyle(.red).padding(12).background(.red.opacity(0.06), in: RoundedRectangle(cornerRadius: 8)) }
                switch tab {
                case "Friends": friendsView
                case "Lists": listsView
                case "Groups": groupsView
                case "Discover": discoveryView
                case "Queue": queueView
                case "Transport": transportView
                default: feedView
                }
            }.padding(30).frame(maxWidth: 1100, alignment: .leading).frame(maxWidth: .infinity)
        }
        .task { await social.reload(model); interests = social.snapshot?.state.preferredSubjects.joined(separator: ", ") ?? ""; applySharingIntent(); await social.recommend(model) }
        .onChange(of: model.sharingBookId) { _, _ in applySharingIntent() }
        .onChange(of: model.sharingDraftId) { _, _ in applySharingIntent() }
        .onChange(of: tab) { _, selected in if selected == "Discover" { Task { await social.recommend(model) } } }
        .onChange(of: social.revealSpoilers) { _, _ in Task { await social.reload(model) } }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
            receiveImportedFile(result, for: fileImport)
        }
        .fileExporter(isPresented: $exporting, document: exportDocument, contentType: .json, defaultFilename: exportName) { result in if case .failure(let error) = result, !bookNookOperationWasCancelled(error) { social.error = bookNookErrorMessage(error) } }
    }
    private func chooseFile(_ kind: SocialFileImport) {
        fileImport = kind
        importing = true
    }
    private func receiveImportedFile(_ result: Result<URL, Error>, for kind: SocialFileImport) {
        Task {
            do {
                let url = try result.get()
                let accessed = url.startAccessingSecurityScopedResource()
                defer { if accessed { url.stopAccessingSecurityScopedResource() } }
                switch kind {
                case .identity:
                    guard (try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0) < 300_000 else { throw AuthorizationError.message("Identity file exceeds 300 KB") }
                    try await model.repository.socialImportBinding(String(contentsOf: url, encoding: .utf8))
                case .history:
                    try await model.repository.socialRestore(url)
                }
                await social.reload(model)
            } catch {
                if !bookNookOperationWasCancelled(error) { social.error = bookNookErrorMessage(error) }
            }
        }
    }
    @ViewBuilder private var networkButton: some View {
        if social.sharingReady {
            Button(state?.outbox.isEmpty == false ? "Send & refresh" : "Refresh shared activity") { Task { await social.exchange(model) } }.buttonStyle(.bordered).disabled(social.busy).help("Send approved outbox records and check for shared activity.")
        } else {
            Button("Set up sharing") { tab = "Transport" }.buttonStyle(.bordered).disabled(social.busy)
        }
    }
    private func applySharingIntent() {
        if let id = model.sharingDraftId {
            guard let draft = model.state?.drafts[id] else {
                social.error = "This private sharing draft is no longer available. Return to its history and prepare it again."
                return
            }
            selectedBook = ""; bodyText = draft.body; stars = 0; kind = "note"
            spoiler = draft.spoiler; tab = "Feed"; composerExpanded = true
            social.status = "Private selection loaded for review. Edit it here; nothing has been published. Its saved copy remains in Unpublished drafts."
            model.sharingBookId = nil
            model.sharingDraftId = nil
            return
        }
        guard let id = model.sharingBookId, let book = model.state?.books[id] else { return }
        selectedBook = id; bodyText = book.review; stars = book.rating ?? 0
        readingStatus = ["to-read", "currently-reading", "read", "paused", "abandoned"].contains(book.shelf) ? book.shelf : "to-read"
        kind = book.review.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && book.rating != nil ? "rating" : "review"
        spoiler = false; tab = "Feed"; composerExpanded = true
        model.sharingBookId = nil
    }

    private var feedView: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack { Text("Newest first").font(.headline); Spacer(); Toggle("Reveal spoilers", isOn: $social.revealSpoilers).toggleStyle(.switch).fixedSize(); networkButton }
            DisclosureGroup("Write a review, rating, update, or selected note", isExpanded: $composerExpanded) { composer(groupID: nil) }.padding(18).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
            if social.snapshot?.feed.isEmpty != false { Text("When you and a friend both accept, their published activity appears here. Group conversations also appear while you are a member.").foregroundStyle(.secondary).padding(.vertical, 24) }
            ForEach(social.snapshot?.feed ?? []) { item in SocialFeedCard(item: item, social: social, model: model) }
        }
    }
    private func canPublish(groupID: String?) -> Bool {
        guard !social.busy, !social.actor.isEmpty else { return false }
        if let groupID { return state?.groups[groupID] != nil && !bodyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        if kind == "rating" { return !selectedBook.isEmpty && stars > 0 }
        if kind == "review" { return !selectedBook.isEmpty && !bodyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        if kind == "reading_update" { return !selectedBook.isEmpty }
        return !bodyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
    @ViewBuilder private func composer(groupID: String?) -> some View {
        let targetGroup = groupID.flatMap { state?.groups[$0] }
        let isPublic = groupID == nil || targetGroup?.private == false
        let count = targetGroup.map { Set($0.members).subtracting($0.banned).count }
        VStack(alignment: .leading, spacing: 12) {
            Label(socialAudience(isPublic: isPublic, recipientCount: count), systemImage: isPublic ? "globe" : "lock.fill").font(.callout.weight(.semibold))
                .accessibilityIdentifier("social-composer-audience")
            if groupID == nil { Picker("Share", selection: $kind) { Text("Selected note").tag("note"); Text("Review").tag("review"); Text("Rating").tag("rating"); Text("Reading update").tag("reading_update") } }
            Picker("Book", selection: $selectedBook) { Text(groupID == nil && ["review", "rating", "reading_update"].contains(kind) ? "Choose a book" : "No book").tag(""); ForEach(model.books) { Text($0.title).tag($0.id) } }
            if groupID == nil && (kind == "rating" || kind == "review") {
                Picker("Stars", selection: $stars) { Text(kind == "review" ? "No rating" : "Choose a rating").tag(0); ForEach(1...5, id: \.self) { Text(String(repeating: "★", count: $0)).tag($0) } }
            }
            if groupID == nil && kind == "reading_update" { Picker("Progress", selection: $readingStatus) { Text("Want to read").tag("to-read"); Text("Reading").tag("currently-reading"); Text("Finished").tag("read"); Text("Paused").tag("paused"); Text("Stopped").tag("abandoned") } }
            TextEditor(text: $bodyText).frame(minHeight: 120).accessibilityLabel("Text to publish").overlay(RoundedRectangle(cornerRadius: 5).stroke(.secondary.opacity(0.2)))
            Toggle("Contains spoilers", isOn: $spoiler)
            Text("You will review this exact text and its audience before approving. Your notebook and earlier drafts stay private.").font(.caption).foregroundStyle(.secondary)
            Button("Review & approve publication") { Task { await publishPost(groupID: groupID) } }.buttonStyle(.borderedProminent).disabled(!canPublish(groupID: groupID))
        }.padding(.top, 12)
    }
    private func publishPost(groupID: String?) async {
        let book = model.books.first { $0.id == selectedBook }
        let post: [String: Any] = ["kind": groupID == nil ? kind : "discussion", "book": book.map { ["id": $0.id, "title": $0.title, "author": $0.authors.joined(separator: ", ")] } as Any? ?? NSNull(), "body": bodyText, "stars": groupID == nil && ["rating", "review"].contains(kind) && stars > 0 ? stars as Any : NSNull(), "reading_status": groupID == nil && kind == "reading_update" ? readingStatus as Any : NSNull(), "spoiler": spoiler, "reply_to": NSNull()]
        await social.publish(["op": "publish", "group_id": groupID as Any? ?? NSNull(), "post": post], title: "Publish this reading selection", model)
    }
    private var friendsView: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Friendship is mutual. Each reader approves their own request or acceptance.").foregroundStyle(.secondary)
            HStack { TextField("Friend’s reader identity code", text: $peer).textFieldStyle(.roundedBorder); Button("Request friendship") { Task { await social.publish(["op": "friendship", "peer": peer.trimmingCharacters(in: .whitespacesAndNewlines), "requested": true], title: "Request mutual friendship", model) } }.disabled(social.busy || social.actor.isEmpty || peer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
            ForEach(social.snapshot?.bindings.filter { $0.holder != social.actor } ?? []) { binding in
                let outgoing = state?.friendshipIntents["\(social.actor):\(binding.holder)"]?.requested == true
                let incoming = state?.friendshipIntents["\(binding.holder):\(social.actor)"]?.requested == true
                VStack(alignment: .leading, spacing: 10) {
                    Text(binding.holder).font(.caption.monospaced()).textSelection(.enabled)
                    Text(outgoing && incoming ? "Mutual friends" : incoming ? "Invited you to be friends" : outgoing ? "Waiting for their acceptance" : "Connected identity").foregroundStyle(.secondary)
                    HStack {
                        Button(outgoing ? "End friendship" : incoming ? "Accept friendship" : "Request friendship") { Task { await social.publish(["op": "friendship", "peer": binding.holder, "requested": !outgoing], title: outgoing ? "End mutual friendship" : "Approve friendship", model) } }
                        Spacer()
                        let blocked = state?.blocked.contains(binding.holder) == true
                        Button(blocked ? "Unblock locally" : "Block locally") { Task { await social.local(["op": "block", "actor": binding.holder, "blocked": !blocked], model) } }
                    }.disabled(social.busy)
                }.padding(18).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
            }
            networkButton
        }
    }
    private var listsView: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Shared reading lists").font(.title2)
            ForEach(sharedLists) { list in
                VStack(alignment: .leading, spacing: 10) {
                    HStack { Text(list.name).font(.headline); Spacer(); Text("v\(list.version)").font(.caption).foregroundStyle(.secondary) }
                    Text(list.description).foregroundStyle(.secondary)
                    ForEach(list.books) { book in Text("\(book.title) · \(book.author)").font(.callout) }
                    if list.owner == social.actor { HStack { Button("Edit list") { selectedList = list.id; name = list.name; description = list.description; selectedBooks = Set(list.books.map(\.id)) }; Button("Delete shared list", role: .destructive) { Task { await social.publish(["op": "delete_list", "id": list.id], title: "Delete this shared reading list", model) } } }.disabled(social.busy) }
                    else { Text("Managed by \(socialShort(list.owner))").font(.caption) }
                }.padding(18).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
            }
            DisclosureGroup(selectedList.isEmpty ? "Create a shared list" : "Edit your selected list") {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("List name", text: $name).textFieldStyle(.roundedBorder); TextField("Description", text: $description).textFieldStyle(.roundedBorder)
                    ForEach(model.books) { book in Toggle(book.title, isOn: Binding(get: { selectedBooks.contains(book.id) }, set: { if $0 { selectedBooks.insert(book.id) } else { selectedBooks.remove(book.id) } })) }
                    HStack { Button("New list instead") { selectedList = ""; name = ""; description = ""; selectedBooks = [] }; Spacer(); Button("Review & publish list") { Task { await publishList() } }.buttonStyle(.borderedProminent).disabled(social.busy || name.isEmpty || social.actor.isEmpty) }
                }.padding(.top, 12)
            }
        }
    }
    private func publishList() async {
        do {
            let id = selectedList.isEmpty ? try await model.repository.socialOwnedID() : selectedList
            let books = model.books.filter { selectedBooks.contains($0.id) }.map { ["id": $0.id, "title": $0.title, "author": $0.authors.joined(separator: ", ")] }
            await social.publish(["op": selectedList.isEmpty ? "create_list" : "update_list", "id": id, "name": name, "description": description, "books": books], title: "Publish this shared reading list", model)
        } catch { social.error = bookNookErrorMessage(error) }
    }
    private var groupsView: some View {
        VStack(alignment: .leading, spacing: 18) {
            DisclosureGroup("Create a reading group") {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("Group name", text: $name).textFieldStyle(.roundedBorder); TextField("Description", text: $description).textFieldStyle(.roundedBorder); Toggle("Private invitations and discussions", isOn: $privateGroup)
                    Button("Review & create group") { Task { do { let id = try await model.repository.socialOwnedID(); await social.publish(["op": "create_group", "id": id, "name": name, "description": description, "private": privateGroup], title: "Create this reading group", model); selectedGroup = id } catch { social.error = bookNookErrorMessage(error) } } }.disabled(name.isEmpty || social.busy || social.actor.isEmpty)
                }.padding(.top, 12)
            }
            ForEach((state?.invitations.values.map { $0 } ?? []).filter { $0.invitee == social.actor && !$0.accepted && $0.expiresAt > UInt64(Date().timeIntervalSince1970) }.sorted { $0.id < $1.id }) { invitation in
                HStack { Label("Invitation: \(state?.groups[invitation.groupId]?.name ?? "Reading group")", systemImage: "envelope"); Spacer(); Button("Accept invitation") { Task { await acceptInvitation(invitation) } }.disabled(social.busy) }
            }
            Picker("Reading group", selection: $selectedGroup) { Text("Select a group").tag(""); ForEach(groups) { Text($0.name + ($0.private ? " · private" : " · public")).tag($0.id) } }
            if let group { groupDetails(group) }
        }
    }
    @ViewBuilder private func groupDetails(_ group: SocialGroup) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(group.name).font(.system(.title2, design: .serif)); Text(group.description).foregroundStyle(.secondary)
            Label("\(group.private ? "Private group" : "Public group") · \(group.members.count) members", systemImage: group.private ? "lock" : "globe").font(.caption)
            if group.members.contains(social.actor) && !group.banned.contains(social.actor) {
                DisclosureGroup("Write to this group") { composer(groupID: group.id) }
                ForEach((social.snapshot?.feed ?? []).filter { $0.groupId == group.id }) { SocialFeedCard(item: $0, social: social, model: model) }
            } else if !group.private && !group.closed && !group.banned.contains(social.actor) {
                let requested = state?.joinRequests.values.contains { $0.groupId == group.id && $0.reader == social.actor && $0.epoch == group.epoch && $0.expiresAt > UInt64(Date().timeIntervalSince1970) } ?? false
                Text(requested ? "Your public membership request is waiting for the owner to admit you." : "Request membership to join this public discussion. Your request and membership will be public.").font(.callout).foregroundStyle(.secondary)
                Button(requested ? "Request pending" : "Review & request membership") { Task { await social.publish(["op": "request_join", "group_id": group.id], title: "Request public group membership", model) } }.disabled(requested || social.busy)
            } else { Text("Accept a current invitation and wait for the owner to admit you before joining the discussion.").font(.callout).foregroundStyle(.secondary) }
            if group.owner == social.actor {
                HStack { TextField("Reader identity code to invite", text: $invitee).textFieldStyle(.roundedBorder); Button("Invite") { Task { do { await social.publish(["op": "invite", "id": UUID().uuidString.lowercased(), "group": try socialObject(group), "invitee": invitee.trimmingCharacters(in: .whitespacesAndNewlines), "expires_at": UInt64(Date().timeIntervalSince1970) + 7 * 86400], title: "Invite a reader to this group", model) } catch { social.error = bookNookErrorMessage(error) } } }.disabled(social.busy) }
                let waiting = waitingReaders(group)
                if !waiting.isEmpty {
                    Text("Waiting for admission: " + waiting.map(socialShort).joined(separator: ", ")).font(.callout)
                    Button("Admit all \(waiting.count) approved readers") { Task { await setMembers(group, members: group.members + waiting, moderators: group.moderators) } }.disabled(social.busy)
                    ForEach(waiting, id: \.self) { reader in
                        HStack { Text(socialShort(reader)).font(.caption.monospaced()); Spacer(); Button("Admit this reader") { Task { await setMembers(group, members: group.members + [reader], moderators: group.moderators) } }.disabled(social.busy) }
                    }
                    Text("Each membership change expires other outstanding approvals. Admit everyone together when appropriate.").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(group.members, id: \.self) { member in
                    HStack {
                        Text(socialShort(member)).font(.caption.monospaced()); if member == group.owner { Text("Owner").font(.caption) }; Spacer()
                        if member != group.owner {
                            let moderator = group.moderators.contains(member)
                            Button(moderator ? "Remove moderator" : "Make moderator") { Task { await setMembers(group, members: group.members, moderators: moderator ? group.moderators.filter { $0 != member } : group.moderators + [member]) } }
                            Button("Remove member", role: .destructive) { Task { await setMembers(group, members: group.members.filter { $0 != member }, moderators: group.moderators.filter { $0 != member }) } }
                        }
                    }.disabled(social.busy)
                }
                Button("Close group", role: .destructive) { Task { await social.publish(["op": "close_group", "group_id": group.id], title: "Close this reading group", model) } }.disabled(social.busy)
            }
        }.padding(18).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
    }
    private func acceptInvitation(_ invitation: SocialInvitation) async {
        if state?.groups[invitation.groupId]?.private == false {
            await social.publish(["op": "request_join", "group_id": invitation.groupId], title: "Request public group membership", model)
        } else {
            await social.publish(["op": "accept_invitation", "invitation_id": invitation.id], title: "Accept this group invitation", model)
        }
    }
    private func waitingReaders(_ group: SocialGroup) -> [String] {
        let now = UInt64(Date().timeIntervalSince1970)
        let candidates: [String]
        if group.private {
            candidates = (state?.invitations.values.map { $0 } ?? []).filter { $0.groupId == group.id && $0.accepted && $0.epoch == group.epoch && $0.expiresAt > now }.map(\.invitee)
        } else {
            candidates = (state?.joinRequests.values.map { $0 } ?? []).filter { $0.groupId == group.id && $0.epoch == group.epoch && $0.expiresAt > now }.map(\.reader)
        }
        return Array(Set(candidates.filter { !group.members.contains($0) && !group.banned.contains($0) })).sorted()
    }
    private func setMembers(_ group: SocialGroup, members: [String], moderators: [String]) async { await social.publish(["op": "set_members", "group_id": group.id, "members": Array(Set(members)).sorted(), "moderators": Array(Set(moderators)).sorted()], title: "Change this group's membership", model) }
    private var discoveryView: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                Text("Books to consider").font(.title2)
                Spacer()
                if social.recommendationsLoading { ProgressView().controlSize(.small).accessibilityLabel("Finding book suggestions") }
                Button("Refresh recommendations") { Task { await social.recommend(model) } }.disabled(social.recommendationsLoading)
            }
            Text("Book suggestions work offline. They use your interests and library subjects, with verified friends’ public ratings when available. Your notebook and private conversations are never used.").font(.callout).foregroundStyle(.secondary)
            HStack {
                TextField("Subjects you enjoy, separated by commas", text: $interests).textFieldStyle(.roundedBorder)
                Button("Save interests") { Task {
                    let subjects = Array(Set(interests.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty })).sorted()
                    await social.local(["op": "set_subjects", "subjects": subjects], model)
                    await social.recommend(model)
                } }.disabled(social.recommendationsLoading)
            }
            if let error = social.recommendationError {
                Text(error).font(.callout).foregroundStyle(.red)
                Button("Try recommendations again") { Task { await social.recommend(model) } }
            } else if social.recommendationsLoaded && social.recommendations.isEmpty && !social.recommendationsLoading {
                VStack(alignment: .leading, spacing: 12) {
                    Text("No suggestions yet").font(.headline)
                    Text(recommendationEmptyMessage).foregroundStyle(.secondary)
                    Button("Open your library") { model.section = "library" }.buttonStyle(.bordered)
                }.padding(18).frame(maxWidth: .infinity, alignment: .leading).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
            }
            ForEach(social.recommendations) { recommendation in
                VStack(alignment: .leading, spacing: 10) {
                    let book = social.catalog[recommendation.bookId]
                    Text(book?.title ?? recommendation.bookId).font(.headline).textSelection(.enabled)
                    if let author = book?.author, !author.isEmpty { Text(author).font(.caption).foregroundStyle(.secondary) }
                    ForEach(recommendation.reasons, id: \.self) { Text($0).font(.callout).foregroundStyle(.secondary) }
                    if model.state?.books[recommendation.bookId] != nil {
                        Button("Open reading desk") { Task {
                            do {
                                try await model.selectBook(recommendation.bookId)
                                model.section = "desk"
                            } catch { social.recommendationError = bookNookErrorMessage(error) }
                        } }.buttonStyle(.bordered)
                            .accessibilityLabel("Open \(book?.title ?? "this book") in the reading desk")
                    } else {
                        Text("From shared activity. Add this title to your library when you are ready to start a notebook.").font(.caption).foregroundStyle(.secondary)
                        Button("Open your library") { model.section = "library" }.buttonStyle(.bordered)
                    }
                }.padding(18).frame(maxWidth: .infinity, alignment: .leading).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
            }
            Divider().padding(.vertical, 8)
            HStack { Text("Public reading groups").font(.title2); Spacer(); networkButton }
            if !social.sharingReady {
                Text("Set up sharing to find groups and connect with other readers. Your book suggestions above work without it. Private groups are invitation-only.").font(.callout).foregroundStyle(.secondary)
            } else {
                Text("Browse groups shared through your connections. A public group’s discussions are visible to everyone; private groups are invitation-only.").font(.callout).foregroundStyle(.secondary)
                if publicGroups.isEmpty {
                    Text("No public groups saved yet. Refresh shared activity to look for groups, or start one of your own.").foregroundStyle(.secondary)
                    Button("Create a group") { tab = "Groups" }
                }
            }
            if let error = social.error {
                HStack(alignment: .top) { Text(error).font(.callout).foregroundStyle(.red); Spacer(); Button("Dismiss") { social.error = nil } }
            }
            ForEach(publicGroups) { group in
                Button { selectedGroup = group.id; tab = "Groups" } label: {
                    VStack(alignment: .leading, spacing: 6) { Text(group.name).font(.headline); Text(group.description).foregroundStyle(.secondary); Text("Public group").font(.caption) }.frame(maxWidth: .infinity, alignment: .leading).padding(16)
                }.buttonStyle(.bordered)
            }
        }
    }
    private var recommendationEmptyMessage: String {
        if model.books.isEmpty { return "Add a book to your library, then give it a few subjects to start getting suggestions." }
        let unread = model.books.filter { $0.shelf != "read" }
        if unread.isEmpty { return "You’re caught up. Books marked Read stay out of suggestions. Add another book or update its reading status in your library." }
        if state?.preferredSubjects.isEmpty != false { return "Save a few interests above, then add matching subjects to books in your library. Friends’ public ratings can also help after sharing setup." }
        if unread.allSatisfy({ $0.subjects.isEmpty }) { return "Your unread books need subjects. Open your library and choose Subjects on a book to add topics such as nature or mystery." }
        return "No unread books match yet. Try a different interest or update the subjects on your library books."
    }
    private var queueView: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack { Text("Approved outbox · \(state?.outbox.count ?? 0)").font(.title2); Spacer(); networkButton }
            Text("A record remains queued until every intended recipient’s delivery is acknowledged by a relay. Stop retrying removes this device’s delivery copy and keeps your history; it cannot recall records already sent or currently in flight.").font(.callout).foregroundStyle(.secondary)
            ForEach((state?.outbox.values.map { $0 } ?? []).sorted { $0.publication.event.at > $1.publication.event.at }, id: \.publication.event.id) { item in VStack(alignment: .leading, spacing: 7) { Text(item.publication.event.action.object?["op"] as? String ?? "Publication").font(.headline); Text("\(item.acknowledged.count) recipient deliveries acknowledged").font(.caption); Text(item.publication.event.id).font(.caption.monospaced()); Button("Stop retrying delivery", role: .destructive) { Task { await social.local(["op": "cancel_delivery", "event_id": item.publication.event.id], model) } }.disabled(social.busy) }.padding(16).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 10)) }
            if let count = social.snapshot?.pending, count > 0 { Text("\(count) received records await missing history or identity verification. They are retained locally for retry.").font(.caption).foregroundStyle(.orange) }
            Divider(); Text("Unpublished drafts").font(.title2)
            ForEach((state?.drafts.values.map { $0 } ?? []).sorted { $0.at > $1.at }) { event in
                VStack(alignment: .leading, spacing: 10) {
                    Text(event.action.object?["op"] as? String ?? "Draft").font(.headline)
                    if let post = event.action.object?["post"] as? [String: Any] { Text(post["body"] as? String ?? "") }
                    HStack {
                        Button("Review again") { Task { await social.retryDraft(event, model) } }
                        Spacer()
                        Button("Discard") { Task { await social.local(["op": "delete_draft", "id": event.id], model) } }
                    }
                }.disabled(social.busy).padding(16).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 10))
            }
            ForEach((model.state?.drafts.values.map { $0 } ?? []).sorted { $0.id < $1.id }) { draft in
                VStack(alignment: .leading, spacing: 10) {
                    Text("Selected reading note").font(.headline)
                    Text(draft.body)
                    Button("Open selection for review") { model.sharingDraftId = draft.id; applySharingIntent() }.disabled(social.busy)
                }.padding(16).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 10))
            }
        }
    }
    private var transportView: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Sharing setup").font(.title2)
            Text("Connect with friends and reading groups when you are ready. Sharing is optional, and every publication needs your approval. Private discussions are encrypted for their recipients; the network can still see connection keys and timestamps.").foregroundStyle(.secondary)
            if social.actor.isEmpty {
                Label("First, set up your reader identity in Connections.", systemImage: "person.crop.circle")
                Button("Set up reader identity") { model.section = "connections" }.buttonStyle(.borderedProminent)
            } else {
                Label("Reader identity configured", systemImage: "checkmark.circle").foregroundStyle(nookAccent)
            }
            Text("Then enable sharing on this device and choose your network connections in Connections.").font(.callout).foregroundStyle(.secondary)
            HStack { Button(social.sharingReady ? "Renew sharing connection" : "Enable sharing on this device") { Task { await social.bind(model) } }.buttonStyle(.borderedProminent).disabled(social.busy || social.actor.isEmpty); networkButton; Button("Network settings") { model.section = "connections" } }
            HStack { Button("Import a friend’s public identity") { chooseFile(.identity) }; Button("Export my public identity") { Task { do { exportDocument = SocialFileDocument(text: try await model.repository.socialExportBinding(social.actor)); exportName = "booknook-transport-identity.json"; exporting = true } catch { social.error = bookNookErrorMessage(error) } } } }
            ForEach(social.snapshot?.bindings ?? []) { binding in HStack(alignment: .top) { VStack(alignment: .leading, spacing: 5) { Text(socialShort(binding.holder)).font(.headline); Text(binding.nostrPublicKey).font(.caption.monospaced()).textSelection(.enabled); Text(binding.active ? "Bound until \(Date(timeIntervalSince1970: TimeInterval(binding.validUntil)).formatted(date: .abbreviated, time: .omitted))" : "Expired or revoked locally").font(.caption).foregroundStyle(.secondary) }; Spacer(); Button("Revoke locally", role: .destructive) { Task { do { try await model.repository.socialRevokeBinding(binding.holder); await social.reload(model) } catch { social.error = bookNookErrorMessage(error) } } }.disabled(!binding.active) }.padding(14).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 10)) }
            Divider()
            Text("Portable social history").font(.headline)
            Text("This export contains your social drafts and private group history. Keep it private. Transport keys and trusted issuer settings are excluded.").font(.caption).foregroundStyle(.secondary)
            HStack { Button("Export social history") { Task { do { let url = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-social-\(UUID().uuidString).json"); defer { try? FileManager.default.removeItem(at: url) }; try await model.repository.socialExport(url); exportDocument = SocialFileDocument(text: try String(contentsOf: url, encoding: .utf8)); exportName = "booknook-social-history.json"; exporting = true } catch { social.error = bookNookErrorMessage(error) } } }; Button("Restore into empty social desk") { chooseFile(.history) } }
        }
    }
}
private struct SocialFeedCard: View {
    let item: SocialFeedItem
    @ObservedObject var social: SocialController
    let model: DeskModel
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text(socialShort(item.actor)).font(.caption.monospaced()); Text(item.post.kind.replacingOccurrences(of: "_", with: " ")).font(.caption).foregroundStyle(.secondary); Spacer(); Text(Date(timeIntervalSince1970: TimeInterval(item.at)).formatted()).font(.caption).foregroundStyle(.secondary) }
            if let book = item.post.book { Text(book.title).font(.system(.title3, design: .serif)) }
            if let stars = item.post.stars { Text(String(repeating: "★", count: stars)).foregroundStyle(nookAccent) }
            if item.spoilerHidden { Label("Spoiler hidden", systemImage: "eye.slash").foregroundStyle(.secondary) } else { Text(item.post.body).textSelection(.enabled) }
            if let reading = item.post.readingStatus { Text(reading.replacingOccurrences(of: "-", with: " ")).font(.caption) }
            HStack {
                Button("Hide for me") { Task { await social.local(["op": "hide", "event_id": item.id, "hidden": true], model) } }
                if item.actor == social.actor { Button("Retract", role: .destructive) { Task { await social.publish(["op": "retract", "event_id": item.id], title: "Retract this published selection", model) } } }
                if let id = item.groupId, let group = social.snapshot?.state.groups[id], group.owner == social.actor || group.moderators.contains(social.actor) {
                    Button("Remove from group", role: .destructive) { Task { await social.publish(["op": "moderate", "group_id": id, "event_id": item.id, "ban_actor": NSNull()], title: "Moderate this group discussion", model) } }
                    if item.actor != group.owner { Button("Ban author", role: .destructive) { Task { await social.publish(["op": "moderate", "group_id": id, "event_id": item.id, "ban_actor": item.actor], title: "Ban this group participant", model) } } }
                }
            }.font(.caption).disabled(social.busy)
        }.padding(20).frame(maxWidth: .infinity, alignment: .leading).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
    }
}
struct SocialFileDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    var text: String
    init(text: String) { self.text = text }
    init(configuration: ReadConfiguration) throws { text = String(decoding: configuration.file.regularFileContents ?? Data(), as: UTF8.self) }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper { FileWrapper(regularFileWithContents: Data(text.utf8)) }
}
