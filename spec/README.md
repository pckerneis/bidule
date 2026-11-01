# Bidule 01 specifications

| Document | What it settles |
| --- | --- |
| [Technical specification](technical-specification.md) | Hardware profile, display and palette, input and audio, storage and cart format, asset sections and editing API, runtime and VM, the cart language, peer exchange, security requirements, conformance |
| [Design document](design-document.md) | Why Bidule exists, goals and non-goals, target platforms, hardware direction, creation and sharing, authoring workflow |
| [Modèle de menaces et architecture de sécurité](threat-model.fr.md) | Threat model and security architecture (in French): cart isolation, implementation, identity and exchange protocol, decentralised moderation, firmware |

Requirements are tagged **Fixed**, **Proposed** or **TBD**, and use RFC 2119 keywords (see *Status and conventions* in the technical specification).

These files are the source of truth for the spec. Change them through commits like the rest of the repo, so a spec change and the emulator change that follows it can land together. Diagrams are Mermaid blocks, which GitHub renders.
