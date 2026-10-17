// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct LibraryView: View {
    @EnvironmentObject var model: DeskModel
    @State var query = ""
    @State var editing: LibraryBook?
    @State var editingSubjects: LibraryBook?
    @State var makingList = false
    @State var editingList: ReadingList?
    private var filteredBooks: [LibraryBook] { model.books.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) || $0.authors.joined(separator: ", ").localizedCaseInsensitiveContains(query) } }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Text("Your library").font(.system(size: 34, design: .serif))
                Text("A record of books, and of the reader you become.").foregroundStyle(.secondary)
                TextField("Find a book or author", text: $query).textFieldStyle(.roundedBorder)
                if filteredBooks.isEmpty {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(model.books.isEmpty ? "Your library starts here" : "No matching books").font(.headline)
                        Text(model.books.isEmpty ? "Add your first book from the sidebar, or import a library from the toolbar." : "Try a different title or author.").foregroundStyle(.secondary)
                        if !query.isEmpty { Button("Clear search") { query = "" } }
                    }.padding(.vertical, 18)
                }
                ForEach(filteredBooks) { book in
                    HStack(spacing: 18) {
                        Image(systemName: "book.closed.fill").font(.largeTitle).foregroundStyle(nookAccent.opacity(0.5)).frame(width: 55)
                        VStack(alignment: .leading, spacing: 7) { Text(book.title).font(.system(.title3, design: .serif)); Text(book.authors.joined(separator: ", ")).font(.callout).foregroundStyle(.secondary); HStack { Text(book.shelf.replacingOccurrences(of: "-", with: " ")).font(.caption); if let rating = book.rating { Text(String(repeating: "★", count: rating)).foregroundStyle(nookAccent) } } }
                        Spacer()
                        Button("Subjects") { editingSubjects = book }.buttonStyle(.bordered)
                        Button("Review & track") { editing = book }.buttonStyle(.bordered)
                        Button("Open desk") { Task { do { try await model.selectBook(book.id); model.section = "desk" } catch { model.error = bookNookErrorMessage(error) } } }.buttonStyle(.borderedProminent)
                    }.padding(18).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
                }
                Divider().padding(.vertical, 10)
                HStack { Text("Reading lists").font(.system(.title2, design: .serif)); Spacer(); Button("New list") { makingList = true } }
                ForEach((model.state?.lists.values.map { $0 } ?? []).sorted { $0.title < $1.title }) { list in
                    VStack(alignment: .leading, spacing: 6) { HStack { Text(list.title).font(.headline); Spacer(); Button("Edit list") { editingList = list } }; Text(list.bookIds.compactMap { model.state?.books[$0]?.title }.joined(separator: " · ")).font(.callout).foregroundStyle(.secondary); Label(list.shared ? "Shared" : "Private · sharing requires Castalia", systemImage: "lock").font(.caption) }.padding(18).frame(maxWidth: .infinity, alignment: .leading).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
                }
            }.padding(36)
        }.sheet(item: $editing) { BookReviewEditor(book: $0) }
        .sheet(item: $editingSubjects) { BookSubjectsEditor(book: $0) }
        .sheet(isPresented: $makingList) { ListEditor() }
        .sheet(item: $editingList) { ListEditor(list: $0) }
    }
}

struct BookSubjectsEditor: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    let book: LibraryBook
    @State private var subjects = ""
    @State private var saving = false
    @State private var failure: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("Subjects in \(book.title)").font(.system(.title2, design: .serif))
            TextField("Subjects, separated by commas", text: $subjects).textFieldStyle(.roundedBorder)
            Text("These private catalog details help match books to your interests. Your notebook is never used for recommendations.").font(.callout).foregroundStyle(.secondary)
            if let failure { Text(failure).foregroundStyle(.red) }
            HStack {
                Button("Cancel") { dismiss() }.disabled(saving)
                Spacer()
                Button("Save subjects") { Task {
                    saving = true; defer { saving = false }
                    do {
                        let values = subjects.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
                        let raw = try JSONSerialization.data(withJSONObject: ["op": "set_book_subjects", "book_id": book.id, "subjects": values])
                        model.accept(try await model.repository.execute(raw)); dismiss()
                    } catch { failure = bookNookErrorMessage(error) }
                } }.buttonStyle(.borderedProminent).disabled(saving)
            }
        }.padding(28).frame(minWidth: 380, idealWidth: 540)
        .onAppear { subjects = book.subjects.joined(separator: ", ") }
    }
}

struct BookReviewEditor: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    let book: LibraryBook
    @State var review = ""
    @State var rating = 0
    @State var shelf = "to-read"
    @State private var saving = false
    @State private var failure: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text(book.title).font(.system(.title2, design: .serif))
            Picker("Reading status", selection: $shelf) { Text("Want to read").tag("to-read"); Text("Reading").tag("currently-reading"); Text("Read").tag("read"); Text("Paused").tag("paused") }
            Picker("Your rating", selection: $rating) { Text("Unrated").tag(0); ForEach(1...5, id: \.self) { Text(String(repeating: "★", count: $0)).tag($0) } }
            TextEditor(text: $review).frame(minHeight: 200).accessibilityLabel("Private book review")
            Text("Your rating and review stay private until you explicitly publish them.").font(.caption).foregroundStyle(.secondary)
            if let failure { Text(failure).foregroundStyle(.red) }
            HStack {
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Review for sharing") { Task { await save(share: true) } }.disabled(review.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && rating == 0)
                Button("Save privately") { Task { await save(share: false) } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
        }.padding(28).frame(minWidth: 380, idealWidth: 580)
        .onAppear { review = book.review; rating = book.rating ?? 0; shelf = book.shelf.isEmpty ? "to-read" : book.shelf }
        .disabled(saving).interactiveDismissDisabled(saving)
    }
    @MainActor private func save(share: Bool) async {
        guard !saving else { return }; saving = true; defer { saving = false }
        do {
            try await model.perform(["op": "update_book", "book_id": book.id, "title": book.title, "shelf": shelf, "rating": rating == 0 ? NSNull() : rating as Any, "review": review])
            if share { model.sharingBookId = book.id; model.section = "community" }
            dismiss()
        } catch { failure = bookNookErrorMessage(error) }
    }
}
struct ListEditor: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    var list: ReadingList? = nil
    @State private var saving = false
    @State private var failure: String?
    @State var title = ""
    @State var selected = Set<String>()
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("A reading list").font(.system(.title2, design: .serif))
            TextField("List title", text: $title).textFieldStyle(.roundedBorder)
            ScrollView { VStack(alignment: .leading) { ForEach(model.books) { book in Toggle(book.title, isOn: Binding(get: { selected.contains(book.id) }, set: { if $0 { selected.insert(book.id) } else { selected.remove(book.id) } })) } } }.frame(minHeight: 160)
            if let failure { Text(failure).foregroundStyle(.red) }
            HStack { Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction); Spacer(); Button("Save list") { Task { await save() } }.disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty).buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction) }
        }.padding(28).frame(minWidth: 380, idealWidth: 520)
        .onAppear { if let list { title = list.title; selected = Set(list.bookIds) } }
        .disabled(saving).interactiveDismissDisabled(saving)
    }
    @MainActor private func save() async {
        guard !saving else { return }; saving = true; defer { saving = false }
        do {
            try await model.perform(["op": "save_list", "id": list?.id as Any? ?? NSNull(), "title": title.trimmingCharacters(in: .whitespacesAndNewlines), "book_ids": Array(selected).sorted(), "expected_version": list?.version as Any? ?? NSNull()])
            dismiss()
        } catch { failure = bookNookErrorMessage(error) }
    }
}

struct CommunityView: View {
    var body: some View { SocialCommunityView() }
}
struct ConnectionsView: View {
    @EnvironmentObject var model: DeskModel
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Text("Your books. Your connections.").font(.system(size: 34, design: .serif))
                Text("The reading desk works offline. Every external source declares what it can provide.").foregroundStyle(.secondary)
                ConnectionSetupView()
                HStack { Label("Local library and notebook", systemImage: "internaldrive"); Spacer(); Text("Available").foregroundStyle(nookAccent) }.padding(20).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
                ForEach(model.gates) { gate in
                    VStack(alignment: .leading, spacing: 10) { HStack { Text(gate.id.replacingOccurrences(of: "-", with: " ").capitalized).font(.headline); Spacer(); Label("Release gate", systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.orange) }; Text(gate.reason ?? "Qualification required").font(.callout).foregroundStyle(.secondary) }.padding(20).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
                }
                Divider()
                Text("Book Nook 0.1 · Development preview").font(.caption)
                Text("Ontology 0.1.0 · Zenith / Castalia mapping 0.6.0\nNo Book Nook server is required.\nLocal saves never request a signature. Sharing and synchronization require explicit consent.").font(.caption).foregroundStyle(.secondary)
            }.padding(36)
        }
    }
}
