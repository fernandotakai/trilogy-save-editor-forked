# Trilogy Save Editor

A terminal editor for Mass Effect Legendary Edition saves on PC. It opens a `.pcsav` and lets you change Shepard's stats: name, level, XP, talent points, credits, resources, paragon, renegade, and difficulty.

Mass Effect 3 Legendary uses the same PC save layout as the last original Mass Effect 3 patch, so those files open too. Original Mass Effect 1 and 2 saves, and Xbox and PS4 saves, are left alone.

```
trilogy-save-editor path/to/Save_0001.pcsav
```

With no path, the program asks for one. `~` is expanded.

| Key | Action |
| --- | --- |
| Up / Down | Move |
| Left / Right | Change a choice (gender, origin, class, difficulty) |
| Enter | Type a name or a number |
| s | Save |
| q | Quit |

The first save copies the untouched file to `Save_0001.pcsav.bak`. Later saves leave that backup in place. Quit without saving and the file on disk is unchanged.

Changing gender, origin, or notoriety also updates the plot flags the game reads for those choices. Class in Mass Effect 1 is shown but not changed, because that resets talents and unequips gear.

Legendary saves on Linux are usually under:

```
~/Documents/BioWare/Mass Effect Legendary Edition/Save/
```

On Steam Proton they are under the game's `compatdata` prefix, in `drive_c/users/steamuser/Documents/BioWare/Mass Effect Legendary Edition/Save/`.

## Build

```sh
cargo build --release
```

The binary is `target/release/trilogy-save-editor`.

The save format code comes from [Karlitos' trilogy-save-editor](https://github.com/KarlitosVII/trilogy-save-editor), licensed under CeCILL-2.1.
