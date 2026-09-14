// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @State private var errorMessage: String?
    @State private var isImporterPresented = false
    @State private var selectedURL: URL?

    var body: some View {
        ZStack(alignment: .bottom) {
            if let selectedURL {
                preview(for: selectedURL)
            } else {
                welcome
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(.red.opacity(0.88), in: Capsule())
                    .padding(20)
            }
        }
        .frame(minWidth: 680, minHeight: 480)
        .background(Color(nsColor: .windowBackgroundColor))
        .fileImporter(
            isPresented: $isImporterPresented,
            allowedContentTypes: UTType.schematicFileTypes,
            allowsMultipleSelection: false,
            onCompletion: handleImport
        )
        .dropDestination(for: URL.self, action: handleDrop)
        .onOpenURL(perform: open)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                if selectedURL != nil {
                    Button("Home", systemImage: "house") {
                        selectedURL = nil
                        errorMessage = nil
                    }
                    .help("Return to the LitematicaQL home screen")
                }

                Button("Open", systemImage: "folder") {
                    isImporterPresented = true
                }
                .keyboardShortcut("o")
                .help("Open a Minecraft schematic")
            }
        }
    }

    private var welcome: some View {
        VStack(spacing: 24) {
            AppIconMark()

            VStack(spacing: 9) {
                Text("LitematicaQL")
                    .font(.system(size: 34, weight: .bold, design: .rounded))
                Text("Interactive Quick Look previews for Minecraft schematics")
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }

            Button("Open Schematic…") {
                isImporterPresented = true
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

            DemoGallery { url in
                open(url)
            }

            VStack(alignment: .leading, spacing: 12) {
                Label("Move LitematicaQL to Applications and open it once.", systemImage: "app.badge")
                Label(
                    "Select any \(LitematicFile.extensionList) file in Finder and press Space.",
                    systemImage: "space"
                )
                Label("Drag to orbit, scroll to zoom, and right-drag to pan.", systemImage: "rotate.3d")
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            .padding(18)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 16))

            Text("Schematics stay on your Mac. The previewer never uses the network.")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .padding(36)
    }

    private func preview(for url: URL) -> some View {
        SchematicWebView(fileURL: url, errorMessage: $errorMessage)
            .id(url)
        .navigationTitle(url.lastPathComponent)
    }

    private func handleImport(_ result: Result<[URL], Error>) {
        switch result {
        case let .success(urls):
            guard let url = urls.first else {
                return
            }
            open(url)
        case let .failure(error):
            errorMessage = error.localizedDescription
        }
    }

    private func handleDrop(_ urls: [URL], _: CGPoint) -> Bool {
        guard let url = urls.first(where: LitematicFile.supports) else {
            errorMessage = LitematicFileError.unsupportedExtension.localizedDescription
            return false
        }

        open(url)
        return true
    }

    private func open(_ url: URL) {
        guard LitematicFile.supports(url) else {
            errorMessage = LitematicFileError.unsupportedExtension.localizedDescription
            return
        }

        errorMessage = nil
        selectedURL = url
    }
}

private struct AppIconMark: View {
    private var icon: NSImage? {
        guard let iconURL = Bundle.main.url(forResource: "LitematicaQL", withExtension: "icns"),
              let source = NSImage(contentsOf: iconURL),
              let representation = source.representations
                  .compactMap({ $0 as? NSBitmapImageRep })
                  .max(by: { $0.pixelsWide < $1.pixelsWide }) else {
            return nil
        }

        let resolved = NSImage(size: representation.size)
        resolved.addRepresentation(representation)
        return resolved
    }

    var body: some View {
        Group {
            if let icon {
                Image(nsImage: icon)
                    .resizable()
                    .scaledToFit()
                    .shadow(color: .black.opacity(0.22), radius: 18, y: 8)
            }
        }
        .frame(width: 112, height: 112)
        .accessibilityLabel("LitematicaQL")
    }
}

/// A demo schematic bundled with the app, keyed by the file format it shows off.
///
/// The gallery discovers these from the bundle rather than listing them in
/// code, so adding a file to `Fixtures/Demos` and rerunning the demo generator
/// is all it takes to add one. Order follows `LitematicFile`'s extension list so
/// the gallery reads the same way the documentation does.
private struct DemoSchematic: Identifiable {
    let url: URL
    let format: String

    var id: String { url.lastPathComponent }
    var title: String { url.deletingPathExtension().lastPathComponent }

    static let all: [DemoSchematic] = {
        let bundled = Bundle.main.urls(forResourcesWithExtension: nil, subdirectory: "Demos") ?? []
        let order = LitematicFile.supportedFileExtensions

        return bundled
            .map { DemoSchematic(url: $0, format: $0.pathExtension.lowercased()) }
            .sorted { left, right in
                let leftIndex = order.firstIndex(of: left.format) ?? order.count
                let rightIndex = order.firstIndex(of: right.format) ?? order.count
                return leftIndex < rightIndex
            }
    }()
}

private struct DemoGallery: View {
    let onSelect: (URL) -> Void

    private let demos = DemoSchematic.all

    var body: some View {
        if !demos.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("Bundled demos")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.secondary)

                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 124), spacing: 10)],
                    spacing: 10
                ) {
                    ForEach(demos) { demo in
                        Button {
                            onSelect(demo.url)
                        } label: {
                            card(for: demo)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Preview \(demo.title), a .\(demo.format) demo")
                    }
                }
            }
            .frame(maxWidth: 560)
        }
    }

    private func card(for demo: DemoSchematic) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(demo.title)
                .font(.callout.weight(.semibold))
                .foregroundStyle(.primary)
            Text(".\(demo.format)")
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(.separator.opacity(0.6))
        )
    }
}
