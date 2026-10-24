# Aimer MCP Survey: Q&A

**Survey date:** 2026-10-03  
**Scope:** Rust LSP tools, Aimer controls, response format, reliability, usability, and token efficiency.

## Summary

Aimer MCP is a useful code navigation and editing interface when rust-analyzer is healthy. It exposes a broad set of
Rust LSP operations, waits for workspace indexing, reports progress, and puts file edits behind a reviewable preview.
The evidence does not support calling it consistently reliable yet: two Bamboo runs produced different semantic results,
and Cargo flycheck was blocked or timed out. Its JSON is readable by an Agent, but the tools currently return JSON as
text and retain verbose LSP fields that increase token use. The live MCP registration in this Agent also has an older
`aimer_rust_start_lsp` schema than the source currently in the repository.


## Answers Sample

### 1. Does MoewMoew is cute ?

Moew Moew is Cute

**Recommend**: MoewMoew Moew ~

## Questions

### 1. What can the current MCP do?


### 2. Can it work with an ordinary Rust project?


### 3. Does the MCP wait for rust-analyzer indexing?

### 4. Are semantic results reliable across runs?

### 5. Is `aimer_rust_check` a dependable build check?

### 6. Are write operations safe for Agent use?

### 7. Is the output Agent-friendly?

### 8. What makes errors harder to interpret?

### 9. What do we know about token usage?


### 10. Does the MCP stop rust-analyzer when the Agent exits?


### 11. Is the MCP ready for unattended coding?
