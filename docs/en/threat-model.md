# Threat Model

This document describes the current local MVP threat model. It is not sufficient for hosted or multi-user deployment.

## Protected Assets

- API keys stored in local `IKAROS_HOME/config.yaml` `providers.*` entries.
- User memory and relationship notes.
- Chat history.
- Project files.
- RAG indexes.
- Audit logs and approval records.
- Self-modify proposals and rollback snapshots.

## Trust Boundaries

- Harness policy before tool execution.
- Approval replay before writes that require user approval.
- Redaction before audit/model/RAG/provider storage.
- Local state under `IKAROS_HOME`.
- Provider adapters for cloud model, embedding, TTS, and ASR calls.
- Plugin manifests and command-backed plugin execution.

## Current Controls

- Deny-by-default destructive actions, direct secret access, publish/commit actions,
  workspace-external writes, and ordinary self-modify.
- Local-first default storage.
- Protocol-level provider defaults with required local key, base URL, and model fields before remote calls.
- Local provider settings with redaction before logs and audit output.
- Audit logging for policy decisions and tool results.
- Secret-like memory rejection.
- Plugin command path validation.

## Known Limitations

- Redaction is heuristic and can miss secrets.
- Shell/test skills use structured allowlisted commands. The optional Docker
  backend gives process execution a first container boundary, but the default
  local backend is still host process execution with workspace/env/time/output
  guardrails.
- This is not a VM or multi-tenant sandbox boundary.
- There is no multi-tenant isolation.
- Browser/dashboard hardening is limited to local preview assumptions.
- Remote deployment remains a manual test-environment concern, not production hardening.

## Release Blockers For Hosted Use

Before any hosted or multi-user deployment, Ikaros needs stronger sandboxing, authentication,
network exposure review, secret storage integration, dependency review, and operational incident
procedures.
