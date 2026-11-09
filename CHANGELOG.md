# Changelog

All notable changes to Aimer are recorded here, one file per release in [`changelogs/`](changelogs/).

Aimer is in a development series (`26.0.0-dev.N`), so any release may contain breaking changes. Entries that break
existing code are marked **Breaking** in the release's *Changes* section.

Each release file groups its entries under the same headings:

- **New Features**: capabilities that did not exist before.
- **Fixes**: bugs that were corrected.
- **Improvement**: speed, memory and polish with no change to the API.
- **Changes**: renames, moves, removals and other things that may need a change in your code.

## Releases

| Release | Notes |
|---------|-------|
| [26.0.0-dev.2](changelogs/26.0.0-dev.2.md) | Audio and video playback, shader effects, Venus async utilities, reverse `Scrollable`, accessibility, remote control and screenshots for debug apps (`aimer_prendre`, `aimer_capture`), and a single v2 drawing API (**Breaking**). |

Releases before `26.0.0-dev.2` have no changelog file. Their changes can be read from the git history of the matching
tag, for example `git log 26.0.0-dev.1`.
