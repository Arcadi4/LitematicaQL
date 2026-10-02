import SwiftUI
import UniformTypeIdentifiers

struct TexturePackSettings: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @State private var isImporterPresented = false
    @State private var highlightedPackID: UUID?
    @State private var packToRemove: TexturePackLibrary.Pack?

    var body: some View {
        Form {
            Section {
                Picker("Texture pack", selection: Binding(
                    get: { texturePacks.loadingID ?? texturePacks.selectedID },
                    set: { texturePacks.selectPack($0) }
                )) {
                    Text("Vanilla").tag(nil as UUID?)
                    ForEach(texturePacks.packs) { pack in
                        Text(pack.name).tag(Optional(pack.id))
                    }
                }
                .pickerStyle(.menu)
                .disabled(!texturePacks.isAvailable)
            } header: {
                Text("Appearance")
            } footer: {
                Text("Applies to all previews in this app. Finder Quick Look uses vanilla textures.")
            }

            Section {
                List(selection: $highlightedPackID) {
                    if texturePacks.packs.isEmpty {
                        Text("No added texture packs")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(texturePacks.packs) { pack in
                        HStack {
                            Text(pack.name)
                                .lineLimit(1)
                            Spacer()
                            if texturePacks.loadingID == pack.id {
                                ProgressView()
                                    .controlSize(.small)
                                    .accessibilityLabel("Loading \(pack.name)")
                            } else if texturePacks.selectedID == pack.id {
                                Image(systemName: "checkmark")
                                    .accessibilityLabel("Active texture pack")
                            }
                        }
                        .tag(pack.id)
                        .help(pack.name)
                        .contextMenu {
                            Button("Use Texture Pack") { texturePacks.selectPack(pack.id) }
                            Button("Remove…", role: .destructive) { packToRemove = pack }
                        }
                    }
                }
                .listStyle(.bordered)
                .frame(height: 180)
                .accessibilityLabel("Saved texture packs")
                .onDeleteCommand(perform: requestRemoval)

                HStack {
                    Button("Add…", systemImage: "plus") { isImporterPresented = true }
                        .disabled(texturePacks.isImporting || !texturePacks.isAvailable)
                        .help("Add Minecraft Java resource-pack ZIPs")

                    Button("Remove…", systemImage: "minus", action: requestRemoval)
                        .disabled(!texturePacks.packs.contains(where: { $0.id == highlightedPackID }))
                        .help("Remove the selected saved texture pack")

                    Spacer()

                    if texturePacks.isImporting || texturePacks.loadingID != nil {
                        ProgressView()
                            .controlSize(.small)
                        Text(texturePacks.isImporting ? "Adding packs…" : "Loading textures…")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
            } header: {
                Text("Saved Texture Packs")
            } footer: {
                Text("Add Minecraft Java resource-pack ZIPs. Copies are saved on your Mac. Missing assets use vanilla resources.")
            }
        }
        .formStyle(.grouped)
        .frame(width: 520, height: 460)
        .navigationTitle("LitematicaQL Settings")
        .fileImporter(isPresented: $isImporterPresented, allowedContentTypes: [.zip],
                      allowsMultipleSelection: true) { result in
            switch result {
            case let .success(urls):
                Task { await texturePacks.importPacks(from: urls) }
            case let .failure(error):
                texturePacks.errorMessage = error.localizedDescription
            }
        }
        .confirmationDialog("Remove “\(packToRemove?.name ?? "")”?", isPresented: Binding(
            get: { packToRemove != nil },
            set: { if !$0 { packToRemove = nil } }
        ), titleVisibility: .visible, presenting: packToRemove) { pack in
            Button("Remove", role: .destructive) {
                texturePacks.removePack(pack.id)
                if !texturePacks.packs.contains(where: { $0.id == pack.id }) {
                    highlightedPackID = nil
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: { pack in
            Text(texturePacks.selectedID == pack.id
                 ? "The saved copy will be removed and previews will use Vanilla. The original ZIP is kept."
                 : "The saved copy will be removed from LitematicaQL. The original ZIP is kept.")
        }
        .alert("Unable to Use Texture Pack", isPresented: Binding(
            get: { texturePacks.errorMessage != nil },
            set: { if !$0 { texturePacks.errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) { texturePacks.errorMessage = nil }
        } message: {
            Text(texturePacks.errorMessage ?? "")
        }
    }

    private func requestRemoval() {
        packToRemove = texturePacks.packs.first(where: { $0.id == highlightedPackID })
    }
}
