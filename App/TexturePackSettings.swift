import SwiftUI
import UniformTypeIdentifiers

struct TexturePackSettings: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @State private var isImporterPresented = false
    @State private var isManagerPresented = false

    private var isBusy: Bool { texturePacks.isImporting || texturePacks.loadingID != nil }

    var body: some View {
        Form {
            Section {
                Picker("Texture pack", selection: Binding(
                    get: { texturePacks.activeID },
                    set: { texturePacks.selectPack($0) }
                )) {
                    Text("Vanilla").tag(nil as UUID?)
                    ForEach(texturePacks.packs) { pack in
                        Text(pack.name).tag(Optional(pack.id))
                    }
                }
                .pickerStyle(.menu)
                .disabled(!texturePacks.isAvailable)

                HStack {
                    Button("Add Texture Packs…") { isImporterPresented = true }
                        .disabled(texturePacks.isImporting)
                    Button("Manage Texture Packs…") {
                        isManagerPresented = true
                    }
                    Spacer()
                    if isBusy {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityLabel(texturePacks.isImporting ? "Adding packs" : "Loading textures")
                    }
                }
            } header: {
                Text("Appearance")
            } footer: {
                Text("Add Minecraft Java resource-pack ZIPs. Packs are saved on your Mac; missing assets use vanilla resources.\n\nApplies to previews in this app. Finder Quick Look uses vanilla textures.")
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
        .sheet(isPresented: $isManagerPresented) {
            TexturePackManager()
        }
        .modifier(TexturePackErrorAlert())
    }
}

/// Presents the store's one pack failure. Settings and the manager sheet each
/// attach it to a different view so a single alert site covers both surfaces.
private struct TexturePackErrorAlert: ViewModifier {
    @EnvironmentObject private var texturePacks: TexturePackStore

    func body(content: Content) -> some View {
        content.alert(texturePacks.alert?.title ?? "", isPresented: .init(
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

            if texturePacks.packs.isEmpty {
                Text("No saved texture packs. Add one with “Add Texture Packs…”.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 80)
            } else {
                // The rows live in their own view so the delete confirmation and the
                // shared error alert attach to different views in the hierarchy.
                SavedPackList(packToRemove: $packToRemove)
                    .frame(minHeight: 80, maxHeight: 320)
            }

            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 480)
        .modifier(TexturePackErrorAlert())
    }
}

private struct SavedPackList: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @Binding var packToRemove: TexturePackLibrary.Pack?

    var body: some View {
        List {
            ForEach(texturePacks.packs) { pack in
                HStack(spacing: 12) {
                    Text(pack.name)
                        .lineLimit(2)
                        .help(pack.name)
                    if texturePacks.selectedID == pack.id {
                        Text("Active")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 8)
                    if texturePacks.loadingID == pack.id {
                        ProgressView()
                            .controlSize(.small)
                            .accessibilityLabel("Loading \(pack.name)")
                    }
                    Button("Remove…", role: .destructive) {
                        packToRemove = pack
                    }
                    .controlSize(.small)
                }
                .padding(.vertical, 2)
            }
        }
        .listStyle(.bordered)
        .alert("Remove “\(packToRemove?.name ?? "")”?", isPresented: Binding(
            get: { packToRemove != nil },
            set: { if !$0 { packToRemove = nil } }
        ), presenting: packToRemove) { pack in
            Button("Remove", role: .destructive) {
                texturePacks.removePack(pack.id)
            }
            Button("Cancel", role: .cancel) {}
        } message: { pack in
            Text(texturePacks.selectedID == pack.id
                 ? "The saved copy will be removed and previews will use Vanilla. The original ZIP is kept."
                 : "The saved copy will be removed from LitematicaQL. The original ZIP is kept.")
        }
    }
}
