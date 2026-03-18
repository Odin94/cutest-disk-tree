# Claude Code Instructions

## Running Cargo commands

There is a bug where Claude Code's shell sets `TMP`/`TEMP` to `$CWD/=`, which causes Cargo to fail when creating temp dirs. Always prefix cargo commands with the correct temp dir:

```bash
TMP="C:/Users/kamme/AppData/Local/Temp" TEMP="C:/Users/kamme/AppData/Local/Temp" cargo <command>
```
