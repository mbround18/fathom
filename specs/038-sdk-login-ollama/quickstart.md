# Quickstart: verifying 038

1. Run the app; open Settings -> AI Assist.
2. GitHub: Sign in, note the code and URL, authorize (tests use a fake device server: `cargo test ai_login::github`). Status shows connected with the account.
3. Claude/OpenAI: paste a key; an invalid key shows the provider's message and nothing is stored. Check `~/.fathom/credentials.json` contains no key; check the OS store entry `fathom / ai-assist/claude`.
4. Legacy migration: place an old plaintext agent key in `~/.fathom/credentials.json`, restart, confirm the one-time notice and that the key is gone from disk.
5. Ollama: `ollama serve` + `ollama pull llama3.2`; open the picker, select a model, ask the assistant a question. Stop Ollama, re-probe: "not running" with instructions.
6. Offline: enable Offline mode; hosted login is refused; an Ollama turn works; `cargo test ai_offline` shows zero non-loopback requests.
7. Checks: `cargo test`, `pnpm --dir ui test`, `pnpm --dir ui e2e -g "ai-assist"`.
