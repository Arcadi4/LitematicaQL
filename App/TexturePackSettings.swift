import SwiftUI
import UniformTypeIdentifiers

struct TexturePackSettings: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @State private var isImporterPresented = false
    @State private var packToRemove: TexturePackLibrary.Pack?
    @State private var bypassesPreviewLimits = RenderLimits.bypassesPreviewLimits()

    var body: some View {
        Form {
            Section {
                TexturePackStackList(packToRemove: $packToRemove)

                HStack {
                    Button("Add Texture Packs…") { isImporterPresented = true }
                        .disabled(texturePacks.isImporting)
                    Spacer()
                    if texturePacks.isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityLabel(texturePacks.isImporting
                                                ? "Adding packs"
                                                : "Loading texture pack stack")
                    }
                }
            } header: {
                Text("Appearance")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    if let failure = texturePacks.failureMessage {
                        Label(failure, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                    }
                    Text("Enabled packs apply from top to bottom. Bundled vanilla is always the lowest-priority base.")
                }
            }

            Section {
                Toggle("Render oversized schematics without asking", isOn: Binding(
                    get: { bypassesPreviewLimits },
                    set: { newValue in
                        bypassesPreviewLimits = newValue
                        RenderLimits.setBypassesPreviewLimits(newValue)
                    }
                ))
                .accessibilityHint("Skips the warning for schematics that exceed the preview limits")
            } header: {
                Text("Preview Limits")
            } footer: {
                Text("Always renders schematics that exceed the preview quotas, in the app and in Quick Look. Very large files may take forever or exhaust memory.")
            }
        }
        .formStyle(.grouped)
        .frame(width: 520)
        .fileImporter(isPresented: $isImporterPresented, allowedContentTypes: [.zip],
                      allowsMultipleSelection: true) { result in
            switch result {
            case let .success(urls):
                Task { await texturePacks.importPacks(from: urls) }
            case let .failure(error):
                texturePacks.reportImportFailure(error)
            }
        }
        .modifier(TexturePackErrorAlert())
    }
}

/// Presents the one pack failure in the store while the settings pane is open.
private struct TexturePackErrorAlert: ViewModifier {
    @EnvironmentObject private var texturePacks: TexturePackStore

    func body(content: Content) -> some View {
        content.alert(texturePacks.alert?.title ?? "", isPresented: .init(
            get: { texturePacks.alert != nil },
            set: { if !$0 { texturePacks.dismissAlert() } }
        ), presenting: texturePacks.alert) { _ in
            Button("OK", role: .cancel) { texturePacks.dismissAlert() }
        } message: { alert in
            Text(alert.message)
        }
    }
}

private struct TexturePackStackList: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @Binding var packToRemove: TexturePackLibrary.Pack?

    var body: some View {
        Group {
            if texturePacks.packs.isEmpty {
                Text("No saved texture packs.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 8)
            } else {
                List {
                    if !texturePacks.enabledPacks.isEmpty {
                        Section {
                            ForEach(texturePacks.enabledPacks) { pack in
                                row(pack, enabled: true)
                            }
                            .onMove { source, destination in
                                texturePacks.moveEnabled(fromOffsets: source,
                                                         toOffset: destination)
                            }
                        } header: {
                            Text("Enabled")
                        }
                    }

                    if !texturePacks.disabledPacks.isEmpty {
                        Section {
                            ForEach(texturePacks.disabledPacks) { pack in
                                row(pack, enabled: false)
                            }
                        } header: {
                            Text("Available")
                        }
                    }
                }
                .listStyle(.bordered)
                .frame(minHeight: 120, maxHeight: 260)
                .accessibilityLabel("Texture pack priority")
            }
        }
        .alert(packToRemove.map { "Remove “\($0.name)”?" } ?? "Remove Texture Pack?",
               isPresented: Binding(
                get: { packToRemove != nil },
                set: { if !$0 { packToRemove = nil } }
               ), presenting: packToRemove) { pack in
            Button("Remove", role: .destructive) {
                texturePacks.removePack(pack.id)
            }
            Button("Cancel", role: .cancel) {}
        } message: { pack in
            Text(removalMessage(for: pack))
        }
    }

    private func row(_ pack: TexturePackLibrary.Pack, enabled: Bool) -> some View {
        let enabledPacks = texturePacks.enabledPacks
        let index = enabled ? enabledPacks.firstIndex(where: { $0.id == pack.id }) : nil
        let canMoveUp = index.map { $0 > 0 } ?? false
        let canMoveDown = index.map { $0 + 1 < enabledPacks.count } ?? false

        return HStack(spacing: 8) {
            Toggle(isOn: Binding(
                get: { texturePacks.isEnabled(pack.id) },
                set: { texturePacks.setEnabled(pack.id, $0) }
            )) {
                Text(pack.name)
                    .lineLimit(1)
            }
            .toggleStyle(.checkbox)
            .help(pack.name)

            Spacer(minLength: 8)

            if enabled {
                Button {
                    texturePacks.moveEnabled(pack.id, by: -1)
                } label: {
                    Image(systemName: "chevron.up")
                }
                .buttonStyle(.borderless)
                .disabled(!canMoveUp)
                .help("Move \(pack.name) up")
                .accessibilityLabel("Move \(pack.name) up")

                Button {
                    texturePacks.moveEnabled(pack.id, by: 1)
                } label: {
                    Image(systemName: "chevron.down")
                }
                .buttonStyle(.borderless)
                .disabled(!canMoveDown)
                .help("Move \(pack.name) down")
                .accessibilityLabel("Move \(pack.name) down")
            }

            Button(role: .destructive) {
                packToRemove = pack
            } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.borderless)
            .help("Remove \(pack.name)…")
            .accessibilityLabel("Remove \(pack.name)")
        }
        .padding(.vertical, 1)
    }

    private func removalMessage(for pack: TexturePackLibrary.Pack) -> String {
        texturePacks.isEnabled(pack.id)
            ? "The saved copy will be removed. Previews will use the remaining texture packs over bundled vanilla. The original ZIP is kept."
            : "The saved copy will be removed from LitematicaQL. The original ZIP is kept."
    }
}
