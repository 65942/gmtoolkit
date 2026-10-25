# GMToolKit

I mostly made this so I wouldn't have to explain to friends how to install CSS textures and use GModPatchTool ever again

It can:
- Download content from other Source games via SteamCMD, even if you don't own them
- Switch Gmod to the x86-64 branch and run GModPatchTool so the Chromium browser works
- Install VPhysics-Box3D or VPhysics-Jolt
- Install SkateGM from your own copy of Skate 3
- Install GWater2
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

SkateGM needs the x86-64 branch, and Windows or GMod running under Proton on Linux. You also need your own Skate 3 (Xbox 360) disc image (`.iso`) or the `default.xex` from an extracted copy. Its maps (Port Carverton and the rest) are built on your PC during install, which takes a while and about 10 GB of free space.

GWater2 also needs its [workshop addon](https://steamcommunity.com/sharedfiles/filedetails/?id=3403736992). On Linux it can only run natively if you use an NVIDIA GPU on the x86-64 branch. With AMD or Intel, running Gmod via proton is required.

You can also use it from the terminal:

```
gmtoolkit tf               # install TF2 content
gmtoolkit tf off           # remove it
gmtoolkit switch x86-64    # switch branch
gmtoolkit chromium         # run the chromium patches
gmtoolkit chromium off     # undo them
gmtoolkit vphysics         # install box3d (vphysics off to undo)
gmtoolkit jolt             # install jolt instead (jolt off to undo)
gmtoolkit skategm skate3.iso   # install skategm (skategm off to undo)
gmtoolkit skategm --maps sgm_skate3_portcarverton   # also: --maps none, --no-addon, --keep-data
gmtoolkit gwater2          # install gwater2 (gwater2 off to undo)
gmtoolkit sync             # fix mounts if something breaks
```

## Building

```
cargo run --release
```

## Credits

- [GModPatchTool](https://github.com/solsticegamestudios/GModPatchTool)
- [VPhysics-Box3D](https://github.com/Asphaltian/VPhysics-Box3D)
- [VPhysics-Jolt](https://github.com/misyltoad/VPhysics-Jolt)
- [SkateGM](https://github.com/the-schwilliam/SkateGM) (`src/vtfenc.rs` is their texture encoder, copied unchanged, GPL-3.0)
- [GWater2](https://github.com/meetric1/gwater2)

## License

GPL-3.0
