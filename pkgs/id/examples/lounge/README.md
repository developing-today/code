# Roc lounge world

The example that uses the server's capability system. The program:

- subscribes to arrivals (`players`) and to a five-second tick
  (`time.tick:5000`);
- greets every arrival by committing a chat line through the `chat.say`
  request — which the world's grant policy may refuse, and the refusal is
  counted and shown in its view (`denied=N`);
- polls the clock through `time.now` on every tick.

It demonstrates the host-event contract (participant 0 delivers JSON), the
request/result loop, subscription refusals, and that grants are journaled.

## Build and run

```sh
ROC=/path/to/roc ZIG=zig ./build.sh
id serve --world --world-admin-token "$WORLD_ADMIN" --world-module lounge.wasm \
    --world-cap players --world-cap time.now
id world invite HOST_NODE --admin-token "$WORLD_ADMIN" --name Ada
id world join HOST_NODE --capability TOKEN      # arrivals greet (after `--grant chat.say`)
id world caps HOST_NODE --capability TOKEN      # grants, wants and usage
```
