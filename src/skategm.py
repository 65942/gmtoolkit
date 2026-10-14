"""Installs SkateGM from its release zip, as SkateGM's own installer (installer/setup_skategm.py) does:
converts the player's own Skate 3 disc with the bundled exporter, then copies the engine module and add-on in.
Run by gmtoolkit through uv; their Setup.exe can't run under Wine (PyInstaller onefile check).

    install.py <release dir> <GarrysMod> <data dir> <data path as GMod sees it> <vgmstream-cli> [<disc>]

Options come as SKATEGM_KEEP, SKATEGM_ADDON, SKATEGM_MAPS and SKATEGM_SKATE2 (see below).

Without a disc it keeps the data converted last time and leaves sounds, music and maps as they are.
On Linux, SKATEGM_WINE (+ WINEPREFIX) runs Source's map tools with GMod's Proton.
"""
import multiprocessing, os, shutil, subprocess, sys, traceback, types
from pathlib import Path

release, gmod, data, vgmstream = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[5])
datapath = sys.argv[4]
disc = Path(sys.argv[6]) if len(sys.argv) > 6 else None
sys.path.insert(0, str(release / 'exporter'))
import convert
from tools.asset_pipeline import skate3_music, skate3_sounds

# Source's map tools are Windows-only: on Linux they run under GMod's Proton, Linux paths as Z:.
# At module level, as the map builder's worker processes (spawned, below) import this file again.
wine = os.environ.get('SKATEGM_WINE')
if wine:
    from mapgen import sourcetools
    z = lambda a: 'Z:' + a if a.startswith('/') else a
    sourcetools.GAMEINFO = sourcetools.GAMEINFO.replace('"%(gmod)s', '"Z:%(gmod)s')
    def run(command, env, **kw):
        env['VPROJECT'] = z(env['VPROJECT'])
        return subprocess.run([wine] + [z(a) for a in command], env=env, **kw)
    sourcetools.subprocess = types.SimpleNamespace(run=run)

if __name__ == '__main__':
    # The map builder starts process pools from its own workers, which deadlocks with Linux's default fork.
    multiprocessing.set_start_method('spawn')
    garrysmod = gmod / 'garrysmod'
    # The options of their installer window: keep converted data, install the add-on, which maps to build.
    keep_data = os.environ.get('SKATEGM_KEEP') == '1'
    addon = os.environ.get('SKATEGM_ADDON', '1') == '1'
    maps = os.environ.get('SKATEGM_MAPS', 'all')
    skate2 = Path(os.environ['SKATEGM_SKATE2']) if os.environ.get('SKATEGM_SKATE2') else None  # only for its maps
    if keep_data:
        print('Skate 3 data: keeping what was converted before', flush=True)
    elif disc:  # converted once per release and disc, so a retry doesn't redo it
        stamp, converted = data / 'converted.txt', f"{os.environ.get('SKATEGM_TAG')}\n{disc.resolve()}\n{disc.stat().st_size}"
        if stamp.is_file() and stamp.read_text(encoding='utf-8') == converted:
            print('Skate 3 data: already converted from this disc, keeping it', flush=True)
        else:
            convert.convert(disc, data)
            stamp.write_text(converted, encoding='utf-8')
    private = data / 'assets' / 'private'
    if not all((private / f).is_file() for f in ('skater.glb', 'game.json', 'stock/physics-skeletons.json')):
        sys.exit('No converted Skate 3 data: install again and choose your Skate 3 disc image (.iso) or default.xex')

    print('Installing the engine module', flush=True)
    (garrysmod / 'lua' / 'bin').mkdir(parents=True, exist_ok=True)
    for name in ('gmcl_skategm_win64.dll', 'skategm_sdl2.dll', 'skategm_gamecontrollerdb.txt'):
        shutil.copy2(release / name, garrysmod / 'lua' / 'bin' / name)
    shutil.rmtree(garrysmod / 'addons' / 'skategm', ignore_errors=True)  # would clash with the Workshop one
    if addon:
        print('Installing the add-on', flush=True)
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

        # The maps (sgm_skate3_portcarverton, ...) into addons/skategm_maps; ones already built are kept.
        # SKATEGM_MAPS: "all", "none" or a comma-separated list.
        # Their texture encoder ships prebuilt only inside Setup.exe. gmtoolkit brings one built from the same
        # source; if this release changed it, it's built here like their textures.py does, minus +crt-static
        # off Windows (rustc drops cdylibs on linux-gnu with it).
        encoder = release / 'exporter' / 'mapgen' / 'native' / 'vtfenc.dll'
        if maps != 'none' and not encoder.is_file() and shutil.which('rustc'):
            subprocess.run(['rustc', '--edition', '2024', '--crate-type', 'cdylib', '-C', 'opt-level=3', '-C', 'panic=abort',
                            *(['-C', 'target-feature=+crt-static'] if os.name == 'nt' else []),
                            encoder.with_suffix('.rs'), '-o', encoder], check=False)
        if maps == 'none':
            print('Maps: not generated', flush=True)
        elif not encoder.is_file():
            print("Maps were skipped: building them needs Rust (rustc, from rustup.rs) for SkateGM's texture encoder", flush=True)
        else:
            if wine:  # create the Wine prefix once, before parallel builds race to
                subprocess.run([wine, 'wineboot', '-u'], capture_output=True)
            print('Building the maps (a few minutes each)', flush=True)
            try:
                from mapgen import install
                from mapgen.install import install_maps
                # It deletes the decoded districts and disc files when done; keep them (they're keyed on each
                # archive) so a retry or a newer release doesn't decode everything again. `skategm off` removes them.
                keep = {data / 'mapgen' / 'districts', data / 'mapgen' / 'disc'}
                install.shutil = types.SimpleNamespace(**vars(shutil))
                install.shutil.rmtree = lambda p, *a, **kw: None if Path(p) in keep else shutil.rmtree(p, *a, **kw)
                built, failed = install_maps({'skate3': disc, 'skate2': skate2}, gmod, data / 'mapgen', lambda m: print(m, flush=True),
                                             names=None if maps == 'all' else maps.split(','))
                if failed:
                    print('Some maps could not be built: ' + ', '.join(failed), flush=True)
            except Exception as error:
                traceback.print_exc()
                print(f'Maps were skipped: {error or type(error).__name__}', flush=True)
    if not addon:
        print("Subscribe to SkateGM on the Steam Workshop if you haven't: "
              'https://steamcommunity.com/sharedfiles/filedetails/?id=3819058525', flush=True)
    print('Done! Pick the SkateGM gamemode, or in any gamemode run "bind j skategm_toggle" once and press J.', flush=True)
