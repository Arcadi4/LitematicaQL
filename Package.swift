// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "LitematicaQLCore",
    platforms: [.macOS(.v13)],
    products: [
        .library(name: "LitematicaQLCore", targets: ["LitematicaQLCore"]),
    ],
    targets: [
        .target(
            name: "LitematicaQLCore",
            path: "Shared",
            exclude: [
                "NativeSchematic.swift", "MetalMesh.swift", "SchematicMetalRenderer.swift",
                "SchematicMetalViewController.swift", "SchematicShaders.metal",
            ],
            sources: ["LitematicFile.swift"]
        ),
        .testTarget(
            name: "LitematicaQLCoreTests",
            dependencies: ["LitematicaQLCore"],
            path: "Tests/CoreTests"
        ),
    ]
)
