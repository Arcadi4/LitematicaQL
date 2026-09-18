import UniformTypeIdentifiers

extension UTType {
    static let litematic = UTType(importedAs: "moe.arcadia.litematic", conformingTo: .data)
    static let minecraftSchematic = UTType(
        importedAs: "moe.arcadia.minecraft-schematic",
        conformingTo: .data
    )
    static let minecraftStructure = UTType(
        importedAs: "moe.arcadia.minecraft-structure",
        conformingTo: .data
    )

    static let schematicFileTypes: [UTType] = [
        .litematic,
        .minecraftSchematic,
        .minecraftStructure,
    ]
}
