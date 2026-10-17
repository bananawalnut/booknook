// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers
#if os(macOS)
import AppKit
#else
import UIKit
#endif

struct NoteBlockView: View {
    @EnvironmentObject var model: DeskModel
    let notebook: Notebook
    let block: NoteBlock
    var noteNumber: Int = 1
    @State var text = ""
    @State var styles: [InlineStyle] = []
    @State var formatRequest: FormatRequest?
    @State var editorSession = UUID().uuidString
    @State var editorBaseUpdate = ""
    @State var newCharacter = false
    @State var characterName = ""
    @State var selectedCharacter = ""
    @State var sharing = false
    @State var shareText = ""
    @State var notebookHistory = false
    @StateObject private var characterCommit = DraftCommitState()
    @StateObject private var shareCommit = DraftCommitState()
    @StateObject private var flipCommit = DraftCommitState()
    var cards: [Card] { model.state?.cards.values.filter { $0.notebookId == notebook.id && $0.blockId == block.id }.sorted { $0.id < $1.id } ?? [] }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            NotebookInkBlockView(session: model.inkSession(notebook: notebook, block: block),
                                 text: $text, styles: $styles, formatRequest: formatRequest,
                                 whiteboard: block.face == "whiteboard", noteLabel: "Note \(noteNumber)",
                                 contextLabel: block.contextLabel.isEmpty ? nil : block.contextLabel, isFlipping: flipCommit.isSaving,
                                 flip: { Task { await flipCommit.perform({
                try await model.setNoteFace(notebook: notebook.id, block: block.id, face: block.face == "whiteboard" ? "text" : "whiteboard")
            }, onSuccess: {}) } },
                                 addCharacter: { characterCommit.clearError(); newCharacter = true }, changed: { value, marks in
                model.saveText(notebook: notebook.id, block: block.id, session: editorSession,
                               baseUpdate: editorBaseUpdate, text: value, styles: marks)
            }) {
                if block.face != "whiteboard" {
                    Menu("Format text") {
                        ForEach(["bold", "italic", "underline"], id: \.self) { style in
                            Button(style.capitalized) { formatRequest = FormatRequest(style: style) }
                        }
                    }
                }
                Button { notebookHistory = true } label: { Label("Notebook revision history", systemImage: "clock.arrow.circlepath") }
                Button { shareCommit.clearError(); shareText = text; sharing = true } label: { Label("Prepare selected note for sharing", systemImage: "square.and.arrow.up") }.disabled(text.isEmpty)
                Text("\(text.split(whereSeparator: \.isWhitespace).count) words")
            }
            if let error = flipCommit.error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
            ForEach(cards) { card in CharacterCardView(card: card).padding(.leading, 50) }
        }.accessibilityIdentifier("notebook-note-\(block.id)")
        .onAppear {
            if let draft = model.pendingEditorDraft(notebook: notebook.id, block: block.id) { text = draft.text; styles = draft.styles; editorBaseUpdate = draft.baseUpdate; editorSession = draft.session }
            else { text = block.text; styles = block.styles ?? []; editorBaseUpdate = block.update }
        }
        .onChange(of: block.text) { old, new in if text == old { text = new } }
        .onChange(of: block.styles) { old, new in if styles == (old ?? []) { styles = new ?? [] } }
        .sheet(isPresented: $newCharacter) {
            VStack(alignment: .leading, spacing: 18) {
                Text("A character, an evolving understanding").font(.system(.title2, design: .serif))
                Text("Reuse a character from an earlier reading pass, or create a new one. Each card keeps its own recoverable interpretation history.").foregroundStyle(.secondary)
                Picker("Character", selection: $selectedCharacter) {
                    Text("Create a new character").tag("")
                    ForEach((model.state?.entities.values.filter { $0.bookId == notebook.bookId && $0.kind == "character" } ?? []).sorted { $0.name < $1.name }) { character in Text(character.name).tag(character.id) }
                }.disabled(characterCommit.isSaving)
                if selectedCharacter.isEmpty { TextField("Character name", text: $characterName).textFieldStyle(.roundedBorder).disabled(characterCommit.isSaving) }
                if let error = characterCommit.error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
                HStack {
                    Button("Cancel") { newCharacter = false }.disabled(characterCommit.isSaving)
                    Spacer()
                    Button(characterCommit.isSaving ? "Attaching…" : "Attach character card") {
                        // Freeze source context at the action, before a delayed
                        // persistence task can observe a different reading position.
                        let command = Result { try model.characterNoteCommand(notebook: notebook.id, block: block.id,
                            name: characterName, existingCharacterID: selectedCharacter.isEmpty ? nil : selectedCharacter) }
                        Task {
                            await characterCommit.perform({ try await model.perform(command.get()) },
                                onSuccess: { characterName = ""; selectedCharacter = ""; newCharacter = false })
                        }
                    }.disabled(characterCommit.isSaving || (selectedCharacter.isEmpty && characterName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
                }
            }.padding(28).background(readingPaper).foregroundStyle(nookInk).frame(minWidth: 380, idealWidth: 500).interactiveDismissDisabled(characterCommit.isSaving)
        }
        .sheet(isPresented: $notebookHistory) { NotebookHistoryView(notebookId: notebook.id, blockId: block.id) }
        .sheet(isPresented: $sharing) {
            VStack(alignment: .leading, spacing: 18) {
                Text("Choose what to share").font(.title2)
                Text("Only this excerpt will be placed in your sharing queue.").foregroundStyle(.secondary)
                TextEditor(text: $shareText).scrollContentBackground(.hidden).background(readingPaper).frame(minHeight: 180).accessibilityLabel("Note excerpt to share").disabled(shareCommit.isSaving)
                if let error = shareCommit.error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
                HStack {
                    Button("Cancel") { sharing = false }.disabled(shareCommit.isSaving)
                    Spacer()
                    Button(shareCommit.isSaving ? "Saving…" : "Save private draft") { Task {
                        await shareCommit.perform({ try await model.perform(["op": "prepare_share", "kind": "selected_note", "book_id": notebook.bookId, "body": shareText, "spoiler": false, "at": Int(Date().timeIntervalSince1970)]) }, onSuccess: { sharing = false })
                    } }.disabled(shareCommit.isSaving || shareText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }.padding(28).background(readingPaper).foregroundStyle(nookInk).frame(minWidth: 380, idealWidth: 520).interactiveDismissDisabled(shareCommit.isSaving)
        }
    }
}

struct CharacterCardView: View {
    @EnvironmentObject var model: DeskModel
    let card: Card
    @State var editing = false
    @State var history = false
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Image(systemName: "person.text.rectangle"); Text(model.state?.entities[card.characterId]?.name ?? "Character").font(.system(.headline, design: .serif)); Spacer(); Button { history = true } label: { Label("\(card.versions.count)", systemImage: "clock.arrow.circlepath") }.buttonStyle(.borderless).accessibilityLabel("\(model.state?.entities[card.characterId]?.name ?? "Character") history, \(card.versions.count) versions").help("View saved character interpretations") }
            if card.heads.count > 1 { Label("\(card.heads.count) interpretations need review", systemImage: "arrow.triangle.branch").font(.caption).foregroundStyle(.orange) }
            ForEach(card.heads) { version in
                Text(version.interpretation.description.isEmpty ? "What do you notice about this character?" : version.interpretation.description).font(.callout).foregroundStyle(version.interpretation.description.isEmpty ? .secondary : .primary)
                if !version.interpretation.traits.isEmpty { Text(version.interpretation.traits.joined(separator: " · ")).font(.caption).foregroundStyle(nookAccent) }
            }
            Button("Develop this card") { editing = true }.font(.caption).buttonStyle(.borderless)
        }.padding(16).background(nookAccent.opacity(0.07), in: RoundedRectangle(cornerRadius: 12))
        .sheet(isPresented: $editing) { CardEditor(card: card) }
        .sheet(isPresented: $history) { CharacterHistoryView(card: card) }
    }
}

struct CardEditor: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    let card: Card
    @State var description = ""
    @State var traits = ""
    @State var observations = ""
    @State var questions = ""
    @State var relationships: [CharacterRelationship] = []
    @State var relationshipTarget = ""
    @State var relationshipDescription = ""
    @StateObject private var commit = DraftCommitState()
    @State private var reconciliation = CharacterReconciliation(parents: [])
    @State private var initialized = false
    var reviewedVersions: [CardVersion] { card.versions.filter { reconciliation.parents.contains($0.id) } }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Your interpretation of \(model.state?.entities[card.characterId]?.name ?? "this character")").font(.system(.title2, design: .serif))
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if reconciliation.parents.count > 1 {
                        VStack(alignment: .leading, spacing: 12) {
                            Text("Review the pending interpretations").font(.headline)
                            Text("Choose a starting version, then combine or revise its fields. Supporting passages from all reviewed versions stay attached, and every original version remains in history.").font(.callout).foregroundStyle(.secondary)
                            ForEach(Array(reviewedVersions.enumerated()), id: \.element.id) { index, version in
                                VStack(alignment: .leading, spacing: 10) {
                                    Text("Interpretation \(index + 1) · \(Date(timeIntervalSince1970: Double(version.at)).formatted())").font(.subheadline.bold())
                                    Text("Saved by \(version.reader)").font(.caption).foregroundStyle(.secondary)
                                    ForEach(characterHistoryFields(version.interpretation, nameForCharacter: { model.state?.entities[$0]?.name ?? "Character \($0)" })) { field in
                                        VStack(alignment: .leading, spacing: 3) {
                                            Text(field.title).font(.caption.bold()).foregroundStyle(.secondary)
                                            ForEach(Array(field.values.enumerated()), id: \.offset) { _, value in Text(value).font(.callout).textSelection(.enabled) }
                                        }
                                    }
                                    Button(reconciliation.startingVersion == version.id ? "Starting version selected" : "Start from interpretation \(index + 1)") {
                                        if reconciliation.select(version.id) { loadFields(version.interpretation) }
                                    }.disabled(reconciliation.startingVersion != nil)
                                    Divider()
                                }
                            }
                        }
                    }
                    if !card.heads.allSatisfy({ reconciliation.parents.contains($0.id) }) {
                        Text("Another interpretation arrived. This save will preserve it for a separate review.").font(.callout).foregroundStyle(.secondary)
                    }
                    if reconciliation.canSave {
                    TextEditor(text: $description).scrollContentBackground(.hidden).background(readingPaper).frame(minHeight: 150).accessibilityLabel("Character description")
                        .overlay(alignment: .topLeading) { if description.isEmpty { Text("Describe the character…").foregroundStyle(.tertiary).padding(6).allowsHitTesting(false) } }
                    TextField("Traits, separated by commas", text: $traits).textFieldStyle(.roundedBorder)
                    TextField("Observations (one per line)", text: $observations, axis: .vertical).textFieldStyle(.roundedBorder)
                    TextField("Open questions (one per line)", text: $questions, axis: .vertical).textFieldStyle(.roundedBorder)
                    DisclosureGroup("Relationships (\(relationships.count))") {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(Array(relationships.enumerated()), id: \.offset) { index, relationship in
                                HStack { Text("\(model.state?.entities[relationship.target]?.name ?? "Character"): \(relationship.description)").font(.callout); Spacer(); Button { relationships.remove(at: index) } label: { Image(systemName: "minus.circle").accessibilityLabel("Remove relationship") }.buttonStyle(.borderless) }
                            }
                            Picker("Related character", selection: $relationshipTarget) { Text("Choose a character").tag(""); ForEach((model.state?.entities.values.filter { $0.kind == "character" && $0.bookId == model.state?.entities[card.characterId]?.bookId && $0.id != card.characterId } ?? []).sorted { $0.name < $1.name }) { character in Text(character.name).tag(character.id) } }
                            HStack {
                                TextField("Relationship", text: $relationshipDescription)
                                Button("Add relationship") { relationships.append(CharacterRelationship(target: relationshipTarget, description: relationshipDescription)); relationshipTarget = ""; relationshipDescription = "" }.disabled(relationshipTarget.isEmpty || relationshipDescription.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                            }
                        }.padding(.vertical, 8)
                    }
                    Text(model.selection == nil ? "This version retains its reading pass and earlier supporting passages." : "The selected book passage will be attached to this version.").font(.caption).foregroundStyle(.secondary)
                    } else { Text("Select a starting version above to begin your reconciliation.").font(.callout).foregroundStyle(.secondary) }
                }.disabled(commit.isSaving)
            }
            if let error = commit.error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Button("Cancel") { dismiss() }.disabled(commit.isSaving)
                Spacer()
                Button(commit.isSaving ? "Saving…" : reconciliation.parents.count > 1 ? "Save reconciled version" : "Save version") { save() }.buttonStyle(.borderedProminent).disabled(commit.isSaving || !reconciliation.canSave)
            }
        }.padding(28).background(readingPaper).foregroundStyle(nookInk).frame(minWidth: 380, idealWidth: 540, minHeight: 440, idealHeight: 620)
        .interactiveDismissDisabled(commit.isSaving)
        .onAppear {
            guard !initialized else { return }; initialized = true
            reconciliation = CharacterReconciliation(parents: card.heads.map(\.id))
            if let id = reconciliation.startingVersion, let version = card.versions.first(where: { $0.id == id }) { loadFields(version.interpretation) }
        }
    }
    func loadFields(_ value: Interpretation) {
        description = value.description; traits = value.traits.joined(separator: ", "); observations = value.observations.joined(separator: "\n"); questions = value.questions.joined(separator: "\n"); relationships = value.relationships
    }
    func save() {
        guard reconciliation.canSave else { return }
        let split: (String, Character) -> [String] = { value, separator in value.split(separator: separator).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty } }
        var passages = reconciliationPassages(reviewedVersions.map(\.interpretation)).map(\.any)
        if let selection = model.selection { passages.append(selection) }
        let relationshipValues = relationships.map { ["target": $0.target, "description": $0.description] }
        let interpretation: [String: Any] = ["description": description, "traits": split(traits, ","), "relationships": relationshipValues, "observations": split(observations, "\n"), "questions": split(questions, "\n"), "passages": passages]
        Task {
            await commit.perform({ try await model.perform(["op": "save_card", "card_id": card.id, "parents": reconciliation.parents, "interpretation": interpretation, "positions": model.selection.map { [$0] } ?? [], "at": Int(Date().timeIntervalSince1970)]) }, onSuccess: { dismiss() })
        }
    }
}


func readingPositionDescription(_ value: JSONValue) -> String {
    guard let location = value.object else { return "Saved position" }
    switch location["kind"] as? String {
    case "text": return "Book passage: \((location["quote"] as? String ?? "").prefix(120))"
    case "page": return "PDF page \((location["page"] as? NSNumber)?.stringValue ?? "—")"
    case "audio":
        let seconds = ((location["milliseconds"] as? NSNumber)?.intValue ?? 0) / 1000
        return "Audio \(seconds / 3600):\(String(format: "%02d", (seconds / 60) % 60)):\(String(format: "%02d", seconds % 60))\((location["chapter"] as? String).map { " · " + $0 } ?? "")"
    default: return "\(location["provider"] as? String ?? "Source"): \(location["value"] as? String ?? "Saved location")"
    }
}
func readingAlignmentDescription(_ value: JSONValue) -> String {
    switch value.object?["status"] as? String {
    case "confirmed": return "edition alignment confirmed"
    case "candidate": return "possible edition match"
    default: return "edition alignment unconfirmed"
    }
}
