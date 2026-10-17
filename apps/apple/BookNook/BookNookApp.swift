// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers

let readingPaper = Color(red: 247.0 / 255, green: 242.0 / 255, blue: 229.0 / 255)
let nookPaper = readingPaper
let nookInk = Color(red: 41.0 / 255, green: 63.0 / 255, blue: 56.0 / 255)
let nookAccent = Color(red: 0.22, green: 0.39, blue: 0.33)

struct BookNookNavigation {
    var sidebarNeedsReveal = false
    var showSidebar: () -> Void = {}
}

private struct BookNookNavigationKey: EnvironmentKey {
    static let defaultValue = BookNookNavigation()
}

extension EnvironmentValues {
    var bookNookNavigation: BookNookNavigation {
        get { self[BookNookNavigationKey.self] }
        set { self[BookNookNavigationKey.self] = newValue }
    }
}

/// Reopening belongs to the section's existing heading, not another toolbar row.
struct SidebarRevealButton: View {
    @Environment(\.bookNookNavigation) private var navigation
    var body: some View {
        if navigation.sidebarNeedsReveal {
            Button(action: navigation.showSidebar) {
                Image(systemName: "sidebar.left").frame(width: 44, height: 44)
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("Show sidebar")
            .accessibilityIdentifier("show-sidebar")
            .help("Show your library and navigation")
        }
    }
}

private struct BookNookNavigationChrome: ViewModifier {
    func body(content: Content) -> some View {
        #if os(iOS)
        content.toolbar(removing: .sidebarToggle).toolbar(.hidden, for: .navigationBar)
        #else
        content.toolbar(removing: .sidebarToggle)
        #endif
    }
}

#if !BOOKNOOK_TEST
@main struct BookNookApp: App {
    @StateObject private var model = DeskModel()
    var body: some Scene {
        WindowGroup { RootView().environmentObject(model).tint(nookAccent).preferredColorScheme(.light).task { if model.state == nil { await model.open() } } }
        #if os(macOS)
        .defaultSize(width: 1320, height: 860)
        #endif
    }
}
#endif

struct RootView: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var addBook = false
    @State private var importing = false
    @State private var importKind = "goodreads"
    @State private var exporting = false
    @State private var reviewingBackup = false
    @State private var exportDocument: ExportDocument?
    @State private var columnVisibility = NavigationSplitViewVisibility.all
    @State private var compactColumn = NavigationSplitViewColumn.detail
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif
    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility, preferredCompactColumn: $compactColumn) {
            VStack(alignment: .leading, spacing: 22) {
                HStack(spacing: 10) {
                    Image("BookNookMark").resizable().interpolation(.none).scaledToFit()
                        .frame(width: 28, height: 28).clipShape(RoundedRectangle(cornerRadius: 6))
                        .accessibilityHidden(true)
                    Text("Book Nook").font(.system(.title2, design: .serif)).bold().lineLimit(1).minimumScaleFactor(0.8)
                    Spacer(minLength: 0)
                    Button {
                        withAnimation { columnVisibility = .detailOnly; compactColumn = .detail }
                    } label: { Image(systemName: "sidebar.left").frame(width: 44, height: 44) }
                    .buttonStyle(.borderless).accessibilityLabel("Hide sidebar")
                    .accessibilityIdentifier("hide-sidebar").help("Make room for your reading desk")
                }.padding(.top, 4)
                Text("A little room for big ideas.").font(.caption).foregroundStyle(.secondary)
                VStack(spacing: 4) {
                    nav("desk", "Reading desk", "pencil.and.outline")
                    nav("library", "Your library", "books.vertical")
                    nav("community", "Community", "person.2")
                    nav("connections", "Connections", "link")
                    nav("sync", "Sync & recovery", "arrow.triangle.2.circlepath")
                }
                Divider()
                Text("ON YOUR DESK").font(.caption2).tracking(2).foregroundStyle(.secondary)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        ForEach(model.books) { book in
                            Button { Task { do { try await model.selectBook(book.id); model.section = "desk"; showDetailInCompact() } catch { model.error = bookNookErrorMessage(error) } } } label: {
                                HStack(alignment: .top) {
                                    RoundedRectangle(cornerRadius: 3).fill(nookAccent.opacity(0.15)).frame(width: 27, height: 38).overlay(Image(systemName: "book.closed").font(.caption))
                                    VStack(alignment: .leading, spacing: 3) { Text(book.title).lineLimit(2).font(.callout); Text(book.authors.joined(separator: ", ")).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
                                    Spacer(minLength: 0)
                                }.padding(8).background(model.selectedBookId == book.id ? nookAccent.opacity(0.09) : .clear, in: RoundedRectangle(cornerRadius: 8))
                            }.buttonStyle(.plain)
                        }
                    }
                }
                HStack {
                    Button { addBook = true } label: { Label("Add a book", systemImage: "plus") }.buttonStyle(.bordered)
                    Spacer(minLength: 4)
                    Menu {
                        Menu {
                            Button("Goodreads CSV") { importKind = "goodreads"; importing = true }
                            Button("Kindle catalog JSON") { importKind = "kindle"; importing = true }
                            Button("Restore Book Nook export") { importKind = "restore"; importing = true }
                        } label: { Label("Import", systemImage: "square.and.arrow.down") }
                        .accessibilityLabel("Import books or restore backup")
                        .accessibilityIdentifier("library-import")
                        Button { reviewingBackup = true } label: { Label("Export private backup", systemImage: "square.and.arrow.up") }
                            .accessibilityIdentifier("export-private-backup")
                            .disabled(model.saving || model.state == nil)
                    } label: { Image(systemName: "ellipsis").frame(width: 44, height: 44) }
                    .menuStyle(.button).buttonStyle(.borderless).accessibilityLabel("Import and backup")
                    .accessibilityIdentifier("library-actions")
                    .help("Import books or export a private backup")
                }
                Label(model.saving ? "Saving locally…" : "Your writing stays with you", systemImage: model.saving ? "clock" : "lock").font(.caption2).foregroundStyle(.secondary)
            }.padding(20).background(nookPaper).navigationSplitViewColumnWidth(min: 240, ideal: 270, max: 320)
                .modifier(BookNookNavigationChrome())
        } detail: {
            Group {
                switch model.section {
                case "library": LibraryView()
                case "community": CommunityView()
                case "connections": ConnectionsView()
                case "sync": SyncRecoveryView()
                default: if model.book != nil { ReadingDeskView() } else { welcome }
                }
            }.background(nookPaper)
                .environment(\.bookNookNavigation, BookNookNavigation(sidebarNeedsReveal: sidebarNeedsReveal, showSidebar: showSidebar))
                .modifier(BookNookNavigationChrome())
        }
        .navigationSplitViewStyle(.balanced)
        .foregroundStyle(nookInk)
        #if os(macOS)
        .toolbarBackground(nookPaper, for: .windowToolbar)
        #endif
        .sheet(isPresented: $addBook) { AddBookView() }
        .sheet(isPresented: $reviewingBackup) {
            VStack(alignment: .leading, spacing: 20) {
                Text("Export a private backup").font(.system(.title2, design: .serif))
                Text("This unencrypted file includes your library, private notebooks and earlier revisions, character history, annotations, accessible book files, and social history. Anyone with the file can read it.")
                Text("Passwords, keys, credentials, and connection settings are excluded. Choose a location you trust.").foregroundStyle(.secondary)
                Button("Use encrypted device transfer instead") { reviewingBackup = false; model.section = "sync" }
                HStack {
                    Button("Cancel", role: .cancel) { reviewingBackup = false }.keyboardShortcut(.cancelAction)
                    Spacer()
                    Button("Choose backup location") { reviewingBackup = false; Task { await prepareExport() } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                }
            }.padding(28).frame(minWidth: 380, idealWidth: 580)
        }
        .castaliaConsent(model.authorization)
        .onChange(of: model.section) { _, _ in showDetailInCompact() }
        .onChange(of: scenePhase) { _, phase in if phase != .active { Task { do { try await model.drainSaves() } catch { model.error = bookNookErrorMessage(error) } } } }
        .fileImporter(isPresented: $importing, allowedContentTypes: importKind == "goodreads" ? [.commaSeparatedText, .plainText] : [.json]) { result in
            Task {
                do {
                    try await model.drainSaves(); let url = try result.get(); let access = url.startAccessingSecurityScopedResource(); defer { if access { url.stopAccessingSecurityScopedResource() } }
                    if importKind == "restore" { model.accept(try await model.repository.restore(from: url)); model.reloadEditorSessions() }
                    else { model.accept(try await model.repository.importCatalogFile(url, format: importKind)) }
                    if let first = model.books.first { try await model.selectBook(first.id) }
                } catch { if !bookNookOperationWasCancelled(error) { model.error = bookNookErrorMessage(error) } }
            }
        }
        .fileExporter(isPresented: $exporting, document: exportDocument, contentType: .json, defaultFilename: "BookNook-export") { result in if case .failure(let e) = result, !bookNookOperationWasCancelled(e) { model.error = bookNookErrorMessage(e) } }
        .alert("Book Nook", isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })) { Button("OK") { model.error = nil }; if model.saving { Button("Retry save") { model.retrySave() } } } message: { Text(model.error ?? "") }
    }
    private var sidebarNeedsReveal: Bool {
        #if os(iOS)
        columnVisibility == .detailOnly || horizontalSizeClass == .compact
        #else
        columnVisibility == .detailOnly
        #endif
    }
    private func showSidebar() {
        withAnimation { columnVisibility = .all; compactColumn = .sidebar }
    }
    private func showDetailInCompact() {
        #if os(iOS)
        if horizontalSizeClass == .compact { compactColumn = .detail }
        #endif
    }
    func nav(_ id: String, _ title: String, _ icon: String) -> some View {
        Button { model.section = id; showDetailInCompact() } label: { Label(title, systemImage: icon).frame(maxWidth: .infinity, alignment: .leading).padding(10).background(model.section == id ? nookAccent.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 8)) }.buttonStyle(.plain)
    }
    var welcome: some View {
        VStack(alignment: .leading, spacing: 24) {
            HStack(spacing: 12) {
                SidebarRevealButton()
                Image(systemName: "pencil.and.outline").font(.system(size: 42)).foregroundStyle(nookAccent)
            }
            Text("Make room for\nyour next thought.").font(.system(size: 42, weight: .regular, design: .serif))
            Text("Bring a book. Keep a notebook. Follow an idea from its first impression to everything it becomes.").font(.title3).foregroundStyle(.secondary).frame(maxWidth: 470, alignment: .leading)
            Button("Start your reading desk") { addBook = true }.buttonStyle(.borderedProminent).controlSize(.large)
            Text("EPUB · PDF · Goodreads CSV · Kindle catalog").font(.caption).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity, maxHeight: .infinity).padding(40)
    }
    func prepareExport() async {
        do { try await model.drainSaves(); let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".json"); try await model.repository.export(to: url); exportDocument = ExportDocument(data: try Data(contentsOf: url)); try? FileManager.default.removeItem(at: url); exporting = true } catch { model.error = bookNookErrorMessage(error) }
    }
}
struct ExportDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    var data: Data
    init(data: Data) { self.data = data }
    init(configuration: ReadConfiguration) throws { data = configuration.file.regularFileContents ?? Data() }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper { FileWrapper(regularFileWithContents: data) }
}
struct AddBookView: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    @State var title = ""
    @State var author = ""
    @State private var saving = false
    @State private var failure: String?
    @State private var createdBookID: String?
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("A new reading desk").font(.system(.title, design: .serif))
            TextField("Book title", text: $title).textFieldStyle(.roundedBorder)
            TextField("Author", text: $author).textFieldStyle(.roundedBorder)
            Text("You can attach an accessible EPUB or PDF next. Your notebook works with any catalog entry.").font(.callout).foregroundStyle(.secondary)
            if let failure { Text(failure).foregroundStyle(.red) }
            HStack {
                Button("Cancel", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button(createdBookID == nil ? "Add book" : "Open saved book") { Task { await save() } }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(32).frame(minWidth: 360, idealWidth: 480)
        .disabled(saving).interactiveDismissDisabled(saving)
    }
    @MainActor private func save() async {
        guard !saving else { return }; saving = true; defer { saving = false }
        do {
            if createdBookID == nil {
                let result = try await model.perform(["op": "add_book", "title": title.trimmingCharacters(in: .whitespacesAndNewlines), "author": author.trimmingCharacters(in: .whitespacesAndNewlines), "at": Int(Date().timeIntervalSince1970)])
                guard let id = result?.object?["book_id"] as? String else { throw CocoaError(.coderReadCorrupt) }
                createdBookID = id
            }
            if let id = createdBookID { try await model.selectBook(id) }
            model.section = "desk"; dismiss()
        } catch { failure = bookNookErrorMessage(error) }
    }
}
