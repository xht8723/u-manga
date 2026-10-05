"""Collect the licenses accompanying pinned build dependencies into the portable bundle."""
from pathlib import Path
import subprocess,json,os
root=Path(__file__).resolve().parents[1]
# Only resolve the packaged target, instead of fetching unused Android/macOS dependencies
# during an otherwise offline Windows build. Retain notices for all resolved target packages.
target=os.environ.get('CARGO_BUILD_TARGET') or next(
    line.removeprefix('host: ') for line in subprocess.check_output(['rustc','-vV'],text=True).splitlines() if line.startswith('host: ')
)
metadata=json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--locked','--offline','--filter-platform',target],cwd=root))
notices=[f'U-Manga third-party dependency notices\nVersions are pinned by Cargo.lock and pnpm-lock.yaml.\nTarget: {target}\n']
for package in sorted(metadata['packages'],key=lambda p:p['name']):
    folder=Path(package['manifest_path']).parent
    if '.cache' not in str(folder) and not folder.is_relative_to(root/'vendor'):continue
    notices.append(f"\n{'='*72}\n{package['name']} {package['version']} — {package.get('license') or 'See supplied license'}\n{package.get('repository') or ''}\n")
    for path in sorted(folder.iterdir()):
        if path.is_file() and path.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
            notices.append(path.read_text(encoding='utf-8',errors='replace'))
for name in ['svelte','@tauri-apps/api','@tauri-apps/plugin-dialog','lucide-svelte']:
    folder=root/'node_modules'/name
    notices.append(f"\n{'='*72}\nFrontend: {name}\n")
    for path in folder.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
            notices.append(path.read_text(encoding='utf-8',errors='replace'))
(root/'assets/licenses/Dependencies.txt').write_text('\n'.join(notices),encoding='utf-8')
print('Dependency notices written.')
