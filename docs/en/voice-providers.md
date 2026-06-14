# Voice Providers

Voice calls go through `ikaros-voice`.

## Providers

Implemented:

- `mock`: deterministic local TTS/ASR provider for explicit offline tests.
- `openai-compatible`: adapter for `/audio/speech` and `/audio/transcriptions`.

The only cloud voice provider name is `openai-compatible`. It selects the wire
format; the configured remote service must actually expose the requested TTS or
ASR endpoint.

Example:

```yaml
providers:
  tts:
    api_key: "replace-with-your-tts-key"
    base_url: "https://api.example.com/v1"
  asr:
    api_key: "replace-with-your-asr-key"
    base_url: "https://api.example.com/v1"

voice:
  tts:
    provider: openai-compatible
    model: provider-tts-model
    voice: alloy
  asr:
    provider: openai-compatible
    model: provider-asr-model
```

## Safety

- Mock providers require no credentials or network and must be selected explicitly.
- Cloud providers read plaintext keys and base URLs from the local `IKAROS_HOME/config.yaml`.
- TTS text is redacted before provider calls.
- Cloud voice calls are network actions and return approval requests when the active policy gates network access.
- `voice tts --output <path>` is a workspace write and requires approval when policy asks.
- ASR transcript output should not echo the source path.

TTS success output reports provider, format, optional output path, byte length,
and redacted text preview; it does not print raw audio bytes. ASR sends the audio
file as multipart form data and renders only transcript metadata.

Voice schemas carry audio path, format, sample rate, and language metadata. Adapters use provider-supported fields and keep unsupported fields as Ikaros-side metadata.
