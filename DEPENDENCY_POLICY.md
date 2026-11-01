# Dependency Policy

This document is describe about the Aimer(`aimer`) is selects, updates and maintains it's dependencies. it apply to all crates inside the workspace.

## Goal

- Keep dependencies minimal and secure 
- Reduce the attack surface from the chain-supply attacks
- Control over the features, implementations and performance.

## Choosing dependencis

Before adding a new dependency we consider:

- **Requirements** : dependency is based on the requirement of the framework, if the requirement is small, we consider implement it directly.
- **Footprint** : we heavily avoid adding the heavy dependency that will increase compile-time.
- **Optional** : some features is optional for user, so it can be open by user, for example: `markdown`.

## Review and merge

- Any new dependency added must be include in the PR, those are review and make desision by the maintainers.