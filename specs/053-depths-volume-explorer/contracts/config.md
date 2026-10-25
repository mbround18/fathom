# Contract: configuration (053)

| Variable | Config key | Default | Meaning |
|---|---|---|---|
| `DEPTHS_FEATURE_FILES` | `feature_files` | `full` | `off`, `read_only` (list, read, download) or `full` (FR-022). Read at start, like the other `DEPTHS_FEATURE_*` |
| `DEPTHS_HELPER_IMAGE` | `helper_image` | own image ID | image for the volume helper (R2) |
| `DEPTHS_FILES_EDITOR_MAX` | `files_editor_max` | `2MiB` | largest file opened in the editor |
| `DEPTHS_FILES_DOWNLOAD_MAX` | `files_download_max` | `2GiB` | largest file or folder download |
| `DEPTHS_FILES_UPLOAD_MAX` | `files_upload_max` | `1GiB` | largest upload, per file |
| `DEPTHS_FILES_HISTORY_FILE_MAX` | `files_history_file_max` | `10MiB` | larger files get no history |
| `DEPTHS_FILES_HISTORY_VERSIONS` | `files_history_versions` | `20` | versions kept per file |
| `DEPTHS_FILES_HISTORY_TOTAL` | `files_history_total` | `1GiB` | total history size |
| `DEPTHS_FILES_HISTORY_DAYS` | `files_history_days` | `30` | age limit |
| `DEPTHS_FILES_HELPERS` | `files_helpers` | `8` | at most this many helpers at once |

- Sizes take the same units as `DEPTHS_MAX_FILE_BYTES`. An invalid value fails start with the key
  named, like the other settings.
- The WS message cap becomes `max(max_message_bytes(max_file_bytes), editor_max + 64 KiB)`.
- The socket proxy and the Depths container need no change (FR-020). If the proxy denies
  container create, every helper start fails and the reason is shown to admins.
