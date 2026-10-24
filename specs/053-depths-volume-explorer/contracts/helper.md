# Contract: volume helper (053)

## Run argv (fixed in `depths-runtime-cli::helper`; no caller-supplied flags)

```
docker run -i --rm --pull never --network none --read-only --cap-drop ALL
  --security-opt no-new-privileges --user <uid>:<gid>
  --memory 256m --pids-limit 32 --cpus 1 --log-driver none
  --label depths.helper=1 --label depths.helper.stack=<stack>
  --entrypoint /usr/local/bin/depths-volume-helper
  --mount type=volume,src=<name>,dst=/volume[,readonly]      # named volume
  --mount type=bind,src=<abs host path>,dst=/volume[,readonly] # bind
  <image id>
```

- Only the server builds this argv, from a `HelperSource` it derived itself (R10).
  - `<name>` must match Docker's volume-name rule.
  - `<abs host path>` must be absolute, normalised, rated by `bind_reason()` as info or nothing,
    and free of `,` and `=` (`--mount` syntax). Anything else is refused before the argv is built.
- `uid` and `gid` are numbers, never names.
- `readonly` is set unless the op writes.

## Frames (stdin/stdout, `depths-volumes::frame`)

```
frame  = len:u32be kind:u8 body[len]
kind   = 0 Hello | 1 Op (JSON) | 2 Reply (JSON) | 3 Data (bytes) | 4 End | 5 Error (JSON)
max len = 1 MiB
```

- **Handshake**: the helper sends `Hello { proto: 1, openat2: bool, uid, gid }`. If `openat2` is
  false the server reports `helper_unavailable`.
- **Ops**: `Stat`, `List { path, page }`, `Read { path, max }`, `Write { path, base, mode? }` (then
  Data… End), `Create`, `Rename`, `Delete { path, recursive }`, `Size { path, cap }`, `Tar { path }`
  (Data… End), `Untar`, which is not used in this version, and `Put { dir, name, replace }`
  (Data… End).
- **Write replies**: `Reply { fingerprint, prior: bool }`. When `prior` is true, Data frames with
  the replaced content follow, for history (R9).
- **Concurrency**: one op at a time per helper. The pool serialises.
- **Exit codes**: 0 on stdin EOF; 70 on a protocol error, after which the server kills and removes
  the helper.
