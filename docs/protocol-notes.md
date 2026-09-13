# Protocol notes

These notes record the official integration direction for future connection work. They do not describe an implemented backend.

## Codex App Server

The local subscription path will use the official Codex App Server and ChatGPT OAuth flow. For the local stdio transport, `codex app-server` exchanges bidirectional JSONL messages. The client sends an `initialize` request, waits for its response, then sends an `initialized` notification before normal requests. Server-initiated requests require client responses too; this is not a one-way stream of chat text.

Future session work should model backend-owned threads, turns, and items behind UI-facing domain types. Thread creation, listing, reading and resuming establish persistent session navigation. Turns contain streamed items with started/completed lifecycles and incremental output. Approval requests, user interactions, turn errors and diff updates need distinct presentation and routing; they should not all become assistant messages. A workspace can contain several roots, while a turn still needs an explicit working-directory policy.

The locally inspected CLI was Codex 0.149.1. Generate schemas using the installed server's `codex app-server generate-json-schema` command and review version compatibility when implementing the transport. App Server thread storage remains authoritative for Codex sessions; the shell's local workspace UUID is a separate identity. Authentication should use the server's supported account/login flow rather than copying credentials or recreating private endpoints.

Reference: [Codex App Server](https://learn.chatgpt.com/docs/app-server).

## Existing client behavior

Official Codex clients organize work into projects and persistent threads, with
background and long-running work requiring visible progress and a way to return
to the task. Codex Air adopts those workflow needs with its own original native
layout. The first milestone establishes the project boundary before adding
conversation, activity and review surfaces.

References: [Codex projects](https://learn.chatgpt.com/docs/projects),
[Long-running work](https://learn.chatgpt.com/docs/long-running-work).

## Agents API

The developer-platform Agents API is a separate execution path with its own API credentials and billing. Its architecture, sessions, events, tools, approvals, and sandbox model must not be treated as interchangeable with a ChatGPT subscription connection.

Reference: [Agents API architecture](https://developers.openai.com/api/docs/guides/agents-api/architecture).
