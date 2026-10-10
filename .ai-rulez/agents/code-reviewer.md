---
type: Reference
title: Code Reviewer
description: Reviews SQL compilation correctness and generated code quality across all target languages. Validates type mappings between SQL types and language-native types, ensures generated code follows each language's idioms and conventions, and verifies that scythe-lint rules catch common SQL anti-patterns. Reviews fixture coverage and integration test completeness.
x-ai-rulez:
  kind: agent
  id: code-reviewer
  metadata:
    model: sonnet
    name: code-reviewer
---

# code-reviewer
