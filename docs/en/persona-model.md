# Persona Model

Persona is prompt and context input. It is not an execution authority.

## Storage

Default path:

```text
IKAROS_HOME/persona.md
```

The loader preserves markdown and parses common sections such as identity, traits, tone, relationship stance, boundaries, and behavior rules.

## Commands

```bash
ikaros persona show
ikaros persona set --name Ikaros --tone "calm, direct"
ikaros persona reset
```

`persona set` writes only to `IKAROS_HOME/persona.md`, rejects secret-like values, and records an audit event.

## Emotion

Runtime emotion state is small and audit-backed. Current signals map task/chat outcomes to states such as neutral, focused, curious, concerned, confused, and satisfied.

Body renderers read the latest emotion from runtime/audit state. Persona text does not get to set policy or permissions.

## Relationship Memory

Relationship memory is stored as local `Relationship` records and shown through:

```bash
ikaros relationship remember "Prefer short updates" --scope user
ikaros relationship show --scope user
```

Chat can extract clear preferences after redaction and de-duplication, but
automatic observations enter the memory candidate inbox first. Use
`--no-relationship-learning` to disable candidate creation for a turn.

## Boundary

Persona can influence tone, context priority, and prompt wording. It cannot grant access to tools, secrets, code changes, approvals, or provider credentials.
