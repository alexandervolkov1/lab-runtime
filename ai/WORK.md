# Current work — Developer Preview Preparation

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
Repository documentation hygiene: COMPLETE
Developer-preview reference/package work: NOT STARTED
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

## Next work after this task

Developer-preview reference writing remains not started. The next separately
authorized work is:

1. architecture/concepts guide;
2. Application API reference;
3. Recorder/SQLite reference;
4. safety/failure cheat sheet;
5. instrument/component extension guide;
6. getting-started/build/run guide;
7. preview packaging.

Do not begin Arduino, Clojure, Clay, WebSocket or final release-documentation work
automatically.
