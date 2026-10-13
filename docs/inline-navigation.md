# Follow note links in the desktop editor

In Reading view, activate an inline wikilink with a click or Tab followed by Enter/Space.
Live Preview uses the same controls. In Source mode, use Ctrl-click or Ctrl-Enter while the
cursor is on the occurrence; macOS uses Cmd. Ordinary source editing stays available.

Aliases display their authored labels. Embeds and heading/block fragments open the target
file; this slice does not scroll to a heading or render an embedded file. Multiple occurrences
can display the same alias and still refer to different notes. Rust supplies the exact source
ranges and resolved identities, including Unicode and CRLF; the frontend does not resolve links.

Save or Discard unsaved changes before following a link. Bindings disappear while source is
dirty and return on discard or a fresh successful load. Activation revalidates the exact saved
source and target. A changed source, invalid project, missing target or pending recovery journal
refuses without replacing the current viewer or writing files. Reload after resolving the cause.
The normal document load has its existing recovery behavior; following a link never runs recovery.
Late responses also leave newer selections or drafts intact.

Only canonical notes represented as library books can open. Links to configured indexes,
roadmaps or other Markdown show their exact identity and an unavailable explanation; they do
not grant access to arbitrary files. Local Akasha retains its selected-project scope and directs
links to other projects or global knowledge to the Global library. Global can follow all
canonical book identities; the existing editing rules still apply.

Frontmatter, code, comments and escaped links have no activation bindings. Markdown remains
authoritative, and navigation does not rewrite it. Interactive backlink panels remain a later
slice; [bounded CLI inspection](note-links.md) already covers both graph directions.
