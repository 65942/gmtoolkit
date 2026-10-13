"""Installs SkateGM from its release zip, as SkateGM's own installer (installer/setup_skategm.py) does:
converts the player's own Skate 3 disc with the bundled exporter, then copies the engine module and add-on in.
Run by gmtoolkit through uv; their Setup.exe can't run under Wine (PyInstaller onefile check).

    install.py <release dir> <GarrysMod> <data dir> <data path as GMod sees it> <vgmstream-cli> [<disc>]

Without a disc it keeps the data converted last time and leaves sounds and music as they are.
"""
import shutil, sys, traceback
from pathlib import Path

release, gmod, data, vgmstream = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[5])
datapath = sys.argv[4]
disc = Path(sys.argv[6]) if len(sys.argv) > 6 else None
sys.path.insert(0, str(release / 'exporter'))
import convert
from tools.asset_pipeline import skate3_music, skate3_sounds

garrysmod = gmod / 'garrysmod'
if disc:
    convert.convert(disc, data)
private = data / 'assets' / 'private'
if not all((private / f).is_file() for f in ('skater.glb', 'game.json', 'stock/physics-skeletons.json')):
    sys.exit('No converted Skate 3 data: install again and choose your Skate 3 disc image (.iso) or default.xex')

print('Installing the engine module and add-on', flush=True)
(garrysmod / 'lua' / 'bin').mkdir(parents=True, exist_ok=True)
for name in ('gmcl_skategm_win64.dll', 'skategm_sdl2.dll', 'skategm_gamecontrollerdb.txt'):
    shutil.copy2(release / name, garrysmod / 'lua' / 'bin' / name)
shutil.rmtree(garrysmod / 'addons' / 'skategm', ignore_errors=True)
shutil.copytree(release / 'skategm', garrysmod / 'addons' / 'skategm')
(garrysmod / 'data' / 'skategm').mkdir(parents=True, exist_ok=True)
(garrysmod / 'data' / 'skategm' / 'datapath.txt').write_text(datapath, encoding='utf-8')
hud = private / 'gmod_hud'
if (hud / 'runtime' / 'trickdisplay.json').is_file():
    shutil.rmtree(garrysmod / 'data' / 'skategm_hud', ignore_errors=True)
    shutil.copytree(hud, garrysmod / 'data' / 'skategm_hud')

# Optional, like in their installer: without them the stock sounds play and there's no music.
if disc:
    for name, done, build in [
        ('board sounds', lambda: skate3_sounds.installed(gmod, convert.hud_sounds(data)), lambda: convert.build_sounds(disc, data, gmod, vgmstream)),
        ('music', lambda: skate3_music.installed(gmod), lambda: convert.build_music(disc, data, gmod, vgmstream)),
    ]:
        try:
            if done():
                print(f'Skate 3 {name}: already installed, keeping them', flush=True)
            else:
                build()
        except Exception as error:
            traceback.print_exc()
            print(f'Skate 3 {name} were skipped: {error or type(error).__name__}', flush=True)
print('Done! Pick the SkateGM gamemode, or in any gamemode run "bind j skategm_toggle" once and press J.', flush=True)
