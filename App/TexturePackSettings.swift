import SwiftUI
import UniformTypeIdentifiers

struct TexturePackSettings: View {
    @EnvironmentObject private var texturePacks: TexturePackStore
    @State private var isImporterPresented = false

    var body: some View {
        Form {
            Section {
                Picker("Rendering", selection: $texturePacks.mode) {
                    Text("Vanilla").tag(TexturePackStore.Mode.vanilla)
                    if texturePacks.hasCustomPack {
                        Text("Custom").tag(TexturePackStore.Mode.custom)
                    }
                }
                .disabled(texturePacks.isLoading)

                if texturePacks.hasCustomPack {
                    LabeledContent("Custom pack") {
                        Text(texturePacks.name)
                            .lineLimit(2)
                            .help(texturePacks.name)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }

                HStack {
                    if texturePacks.isLoading {
                        ProgressView()
                            .controlSize(.small)
                        Text("Preparing textures…")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Choose Pack…") {
                        isImporterPresented = true
                    }
                }
                .disabled(texturePacks.isLoading)
            } header: {
                Text("Appearance")
            } footer: {
                Text("Choose a Minecraft Java resource-pack ZIP. Missing assets use vanilla resources. Your pack is copied into the app and remembered.")
            }

            Section {
                Text("Texture packs apply to schematic previews in this app. Finder Quick Look uses the default pack.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 480, height: 320)
        .fileImporter(isPresented: $isImporterPresented, allowedContentTypes: [.zip]) { result in
            switch result {
            case let .success(url):
                Task { await texturePacks.importPack(from: url) }
            case let .failure(error):
                texturePacks.errorMessage = error.localizedDescription
            }
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
}

/// SettingsLink provides the system's Settings command and window activation.
struct TexturePackSettingsLink: View {
    var body: some View {
        if #available(macOS 14.0, *) {
            SettingsLink {
                Label("Texture Pack", systemImage: "swatchpalette")
            }
            .help("Choose a custom texture pack")
        } else {
            Button {
                NSApp.sendAction(Selector("showSettingsWindow:"), to: nil, from: nil)
            } label: {
                Label("Texture Pack", systemImage: "swatchpalette")
            }
            .help("Choose a custom texture pack")
        }
    }
}
