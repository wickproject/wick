"""Exercise the installer packaging blocks without downloading or compiling CEF."""

import os
import pathlib
import shutil
import subprocess
import tempfile

script = (pathlib.Path(__file__).resolve().parents[1] / 'install-cef-mac.sh').read_text()
copy = script[script.index('    # Copy CEF framework'):script.index('    echo "  Built')]
links = script[script.index('# Each helper'):script.index('# ── Step 4')]
with tempfile.TemporaryDirectory() as root:
    root = pathlib.Path(root)
    build = root / 'build'
    app = build / 'wick-renderer.app'
    frameworks = app / 'Contents/Frameworks'
    frameworks.mkdir(parents=True)
    sdk = build / 'sdk/Release/Chromium Embedded Framework.framework'
    sdk.mkdir(parents=True)
    (sdk / 'Chromium Embedded Framework').write_text('fixture')
    (frameworks / sdk.name).symlink_to(sdk)
    for suffix in ['', ' (GPU)', ' (Renderer)', ' (Alerts)', ' (Plugin)']:
        dest = frameworks / ('wick Helper' + suffix + '.app') / 'Contents/Frameworks'
        dest.mkdir(parents=True)
        (dest / sdk.name).symlink_to(sdk)
    env = dict(os.environ, APP_DIR=str(app), WICK_DIR=str(build), CEF_DIR='sdk')
    subprocess.run(['bash', '-eu', '-c', copy + links], env=env, check=True)
    assert not (frameworks / sdk.name).is_symlink(), 'top-level framework still points outside bundle'
    # Re-running installation must replace a symlink to a directory itself.
    subprocess.run(['bash', '-eu', '-c', links], env=env, check=True)
    assert not (frameworks / sdk.name / sdk.name).exists(), 'nested framework link created'
    archive = root / 'bundle.tar.gz'
    subprocess.run(['tar', 'czf', str(archive), '-C', str(build), app.name], check=True)
    relocated = root / 'relocated'
    relocated.mkdir()
    subprocess.run(['tar', 'xzf', str(archive), '-C', str(relocated)], check=True)
    shutil.rmtree(build)
    helper_links = list((relocated / app.name / 'Contents/Frameworks').glob(
        'wick Helper*.app/Contents/Frameworks/*.framework'))
    assert len(helper_links) == 5, helper_links
    for link in helper_links:
        assert not pathlib.Path(os.readlink(link)).is_absolute(), link
        assert (link / 'Chromium Embedded Framework').is_file(), link
    # Normalize an old prebuilt bundle whose helper links target a missing CI path.
    env['APP_DIR'] = str(relocated / app.name)
    for link in helper_links:
        link.unlink()
        link.symlink_to('/Users/runner/missing/Chromium Embedded Framework.framework')
    subprocess.run(['bash', '-eu', '-c', links], env=env, check=True)
    assert all((link / 'Chromium Embedded Framework').is_file() for link in helper_links)
    print('PASS: framework and all five helpers survive archive relocation')
