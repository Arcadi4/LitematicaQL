import SwiftUI
import UniformTypeIdentifiers

struct TexturePackSettings: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @State private var isImporterPresented = false
    @State private var isManagerPresented = false

    private var currentPack: TexturePackLibrary.Pack? {
        let id = texturePacks.loadingID ?? texturePacks.selectedID
        return texturePacks.packs.first(where: { $0.id == id })
    }

    var body: some View {
        Form {
            Section {
                LabeledContent("Texture pack") {
                    HStack {
                        if texturePacks.isImporting || texturePacks.loadingID != nil {
                            ProgressView()
                                .controlSize(.small)
                                .accessibilityLabel(texturePacks.isImporting ? "Adding packs" : "Loading textures")
                        }

                        Menu {
                            Picker("Texture pack", selection: Binding(
                                get: { texturePacks.loadingID ?? texturePacks.selectedID },
                                set: { texturePacks.selectPack($0) }
                            )) {
                                Text("Vanilla").tag(nil as UUID?)
                                ForEach(texturePacks.packs) { pack in
                                    Text(pack.name).tag(Optional(pack.id))
                                }
                            }
                            .pickerStyle(.inline)

                            Divider()

                            Button("Add Texture Packs…") { isImporterPresented = true }
                                .disabled(texturePacks.isImporting)
                            Button("Manage Texture Packs…") {
                                isManagerPresented = true
                            }
                        } label: {
                            Text(currentPack?.name ?? "Vanilla")
                                .lineLimit(1)
                        }
                        .menuStyle(.borderlessButton)
                        .menuIndicator(.visible)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityLabel("Texture pack")
                        .accessibilityValue(currentPack?.name ?? "Vanilla")
                        .help(currentPack?.name ?? "Vanilla")
                        .disabled(!texturePacks.isAvailable)
                    }
                }
            } header: {
                Text("Appearance")
            } footer: {
                Text("Add Minecraft Java resource-pack ZIPs. Packs are saved on your Mac; missing assets use vanilla resources.\n\nApplies to previews in this app. Finder Quick Look uses vanilla textures.")
            }
        }
        .formStyle(.grouped)
        .frame(width: 520, height: 220)
        .navigationTitle("LitematicaQL Settings")
        .fileImporter(isPresented: $isImporterPresented, allowedContentTypes: [.zip],
                      allowsMultipleSelection: true) { result in
            switch result {
            case let .success(urls):
                Task { await texturePacks.importPacks(from: urls) }
            case let .failure(error):
                texturePacks.reportImportFailure(error)
            }
        }
        .sheet(isPresented: $isManagerPresented) {
            TexturePackManager()
                .environmentObject(texturePacks)
        }
        .modifier(TexturePackErrorAlert())
    }
}

/// Presents the store's one pack failure. Settings and the manager sheet each
/// attach it to a different view so a single alert site covers both surfaces.
private struct TexturePackErrorAlert: ViewModifier {
    @EnvironmentObject private var texturePacks: TexturePackStore

    func body(content: Content) -> some View {
        content.alert(texturePacks.alert?.title ?? "", isPresented: Binding(
            get: { texturePacks.alert != nil },
            set: { if !$0 { texturePacks.dismissAlert() } }
        )) {
            Button("OK", role: .cancel) { texturePacks.dismissAlert() }
        } message: {
            Text(texturePacks.alert?.message ?? "")
        }
    }
}

private struct TexturePackManager: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @Environment(\.dismiss) private var dismiss
    @State private var packToRemove: TexturePackLibrary.Pack?

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Manage Texture Packs")
                    .font(.title3.weight(.semibold))
                Text("Removing a saved pack keeps the original ZIP.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            Group {
                if texturePacks.packs.isEmpty {
                    VStack(spacing: 6) {
                        Text("No saved texture packs")
                            .fontWeight(.medium)
                        Text("Add a pack from the Texture pack menu.")
                            .font(.callout)
                            .multilineTextAlignment(.center)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(24)
                } else {
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(texturePacks.packs) { pack in
                                HStack(spacing: 16) {
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(pack.name)
                                            .lineLimit(2)
                                            .help(pack.name)
                                        if texturePacks.selectedID == pack.id {
                                            Text("Active")
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)

                                    if texturePacks.loadingID == pack.id {
                                        ProgressView()
                                            .controlSize(.small)
                                            .accessibilityLabel("Loading \(pack.name)")
                                    }

                                    Button("Remove…", role: .destructive) {
                                        packToRemove = pack
                                    }
                                    .buttonStyle(.bordered)
                                    .controlSize(.small)
                                    .accessibilityLabel("Remove \(pack.name)")
                                }
                                .frame(minHeight: 40)
                                .padding(12)

                                if pack.id != texturePacks.packs.last?.id {
                                    Divider()
                                        .padding(.horizontal, 12)
                                }
                            }
                        }
                    }
                    .frame(height: min(CGFloat(texturePacks.packs.count * 65), 260))
                    .accessibilityLabel("Saved texture packs")
                }
            }
            .background(.background, in: RoundedRectangle(cornerRadius: 8))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(.primary.opacity(0.1))
            }

            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 480)
        .onExitCommand { dismiss() }
        .confirmationDialog("Remove “\(packToRemove?.name ?? "")”?", isPresented: Binding(
            get: { packToRemove != nil },
            set: { if !$0 { packToRemove = nil } }
        ), titleVisibility: .visible, presenting: packToRemove) { pack in
            Button("Remove", role: .destructive) {
                texturePacks.removePack(pack.id)
            }
            Button("Cancel", role: .cancel) {}
        } message: { pack in
            Text(texturePacks.selectedID == pack.id
                 ? "The saved copy will be removed and previews will use Vanilla. The original ZIP is kept."
                 : "The saved copy will be removed from LitematicaQL. The original ZIP is kept.")
        }
        .modifier(TexturePackErrorAlert())
    }
}
