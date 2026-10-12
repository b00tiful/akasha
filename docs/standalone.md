# Akasha and external editors

Akasha operates through its shared Rust core, CLI, terminal interface and desktop client.
Its on-demand MCP server serves agent-assisted onboarding only. No Obsidian process,
plugin, REST API, credential or MCP bridge is required or connected.

Markdown with frontmatter and wikilinks remains authoritative. Users can independently
open the same data folder in Obsidian or another editor. That editor's settings, plugins,
cache and UI belong to that application. Akasha treats edits to shared Markdown as external
file changes and applies its existing validation, refresh and conflict safeguards.

Agents read files directly or use bounded `akasha context` and named CLI commands. Humans
use Akasha's own terminal or desktop interface. Development notes under `akasha/` remain a
temporary Markdown workbench accessed directly; they are not automatically migrated into
a configured product root.

Akasha's core already parses and validates its supported exact vault-relative wikilinks
and exposes outgoing note identities. Clickable inline links and bounded backlink lists
inside Akasha are subsequent implementation work. Their absence does not require another
application or justify an external-editor integration.

The former REST/MCP bridge, launcher, fallback logger, dedicated credentials and native
backlink prototype have been removed. Historical development evidence remains archived
in private project memory and is not an installation procedure.
