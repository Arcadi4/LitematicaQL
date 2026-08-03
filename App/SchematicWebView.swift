// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
//
// See the LICENSE file for the full license text.

import SwiftUI

struct SchematicWebView: NSViewControllerRepresentable {
    let fileURL: URL
    @Binding var errorMessage: String?

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSViewController(context: Context) -> SchematicWebViewController {
        SchematicWebViewController()
    }

    func updateNSViewController(
        _ controller: SchematicWebViewController,
        context: Context
    ) {
        context.coordinator.load(
            fileURL,
            in: controller,
            errorMessage: $errorMessage
        )
    }

    static func dismantleNSViewController(
        _: SchematicWebViewController,
        coordinator: Coordinator
    ) {
        coordinator.task?.cancel()
    }

    @MainActor
    final class Coordinator {
        fileprivate var task: Task<Void, Never>?
        private var lastURL: URL?
        private var loadGeneration = 0

        func load(
            _ url: URL,
            in controller: SchematicWebViewController,
            errorMessage: Binding<String?>
        ) {
            guard lastURL != url else {
                return
            }

            lastURL = url
            loadGeneration += 1
            let generation = loadGeneration
            task?.cancel()
            task = Task { @MainActor in
                do {
                    errorMessage.wrappedValue = nil
                    try await controller.preparePreview(of: url)
                } catch is CancellationError {
                    return
                } catch {
                    guard !Task.isCancelled,
                          generation == loadGeneration,
                          url == lastURL else {
                        return
                    }
                    errorMessage.wrappedValue = error.localizedDescription
                }
            }
        }
    }
}
