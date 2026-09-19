# Current work — Developer Preview Preparation

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
Repository documentation hygiene: COMPLETE
Developer Preview Reference: COMPLETE
Preview packaging: NOT STARTED
Practical integration phase: NOT STARTED
```

## Completed preparation step

The repository documentation surface was cleaned before preview-reference writing:

- keep only current developer-facing documentation outside `ai/`;
- keep active coordination concise and current;
- archive only safety rationale and engineering evidence useful for future regression
  investigation;
- delete superseded milestone, POC, migration and obsolete-technology documents;
- preserve accepted artifacts and fix references after moves.

The cleanup changed documentation and repository organization only. It did not modify
production code, tests, Cargo/configuration/schema, API or Runtime/Recorder semantics.

## Completed reference step

The compact public reference now covers architecture/concepts, the complete
Application API, Recorder/SQLite schema and archive semantics, safety/failure
behavior, extension paths, and preview-sufficient build/run instructions.

## Next work after this task

Preview packaging/build is the next separately authorized preparation step.

Do not begin packaging, Arduino, Clojure, Clay, WebSocket, practical integration, or
final release-documentation work automatically.
