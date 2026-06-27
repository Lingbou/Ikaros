# Ikaros Documentation

[Documentation index](../README.md) | [简体中文](../zh-CN/README.md)

This directory contains the English subsystem documentation. The documents are
written as interface notes: each page should explain what the subsystem owns,
which caller context it expects, which data it persists, how failures are
reported, and which invariants must be preserved by future changes.

The style intentionally follows the practical shape of Linux kernel subsystem
documentation: prefer precise contracts over marketing text, keep implementation
details close to the interface that depends on them, and describe limitations as
part of the interface instead of as project history.

## Reading Order

Read these first when changing runtime behavior:

1. [Architecture](architecture.md)
2. [Safety model](safety-model.md)
3. [Harness model](harness-model.md)
4. [Agent loop](agent-loop.md)
5. [Configuration](configuration.md)

Read subsystem documents next, based on the code being changed. Planned future
work belongs in [the root roadmap](../../ROADMAP.md), not in subsystem contract
pages.

## Writing Style

- Write for people first. Start with what the subsystem owns and how callers use
  it.
- Keep overview pages short. Move JSON schemas, protocol lines, and exhaustive
  command output into [API reference](api-reference.md) or the relevant subsystem
  page.
- Prefer short paragraphs, categorized command lists, and stable headings.
- Keep future plans in [the root roadmap](../../ROADMAP.md), not scattered
  through subsystem documents.

## Core Documents

- [Architecture](architecture.md)
- [Safety model](safety-model.md)
- [Harness model](harness-model.md)
- [Agent loop](agent-loop.md)
- [Configuration](configuration.md)
- [API reference](api-reference.md)
- [Threat model](threat-model.md)

## Runtime Subsystems

- [Memory model](memory-model.md)
- [Memory providers](memory-providers.md)
- [Context engine](context-engine.md)
- [RAG model](rag-model.md)
- [Model providers](model-providers.md)
- [Voice providers](voice-providers.md)
- [Persona model](persona-model.md)
- [Body model](body-model.md)
- [Automation model](automation-model.md)
- [Message gateway](message-gateway.md)
- [Service manager templates](service-manager.md)

## Development And Operations

- [Plugin system](plugin-system.md)
- [Self-modify](self-modify.md)
- [Deployment](deployment.md)
