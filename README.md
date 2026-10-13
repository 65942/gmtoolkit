# GMToolKit

A tool I made to set up Garry's Mod without doing everything by hand.

It can:
- Download content from other Source games via SteamCMD, even if you don't own them
- Switch Gmod to the x86-64 branch and run GModPatchTool so the Chromium browser works
- Install VPhysics-Box3D
- On Linux, pick between native and Proton

Works on Linux. Windows and Mac might work but I haven't tested it.

## Supported Game Content

- Counter-Strike: Source
- Team Fortress 2
- Day of Defeat: Source
- Half-Life 2: Deathmatch
- Left 4 Dead
- Left 4 Dead 2
- Counter-Strike: Global Offensive
- Insurgency
- Black Mesa
- No More Room in Hell
- Day of Infamy
- Fistful of Frags
- Zombie Panic! Source
- Pirates, Vikings & Knights II

## Download

There's no release yet. You can get a build from the Actions tab (you need to be logged in). Click the latest run and the files are at the bottom.

## Usage

Open it, pick your Gmod folder if it doesn't find it, and click stuff. Close Steam before switching branches.

You can also use it from the terminal:

```
gmtoolkit tf               # install TF2 content
gmtoolkit tf off           # remove it
gmtoolkit switch x86-64    # switch branch
gmtoolkit chromium         # run the chromium patches
gmtoolkit chromium off     # undo them
gmtoolkit vphysics         # install box3d (vphysics off to undo)
gmtoolkit sync             # fix mounts if something breaks
```

## Building

```
cargo run --release
```

## Credits

- [GModPatchTool](https://github.com/solsticegamestudios/GModPatchTool)
- [VPhysics-Box3D](https://github.com/Asphaltian/VPhysics-Box3D)

## License

GPL-3.0
