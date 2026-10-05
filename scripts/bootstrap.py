"""Build-time asset preparation. Python is not shipped or invoked by U-Manga."""
from pathlib import Path
import argparse, hashlib, io, json, tarfile, urllib.request, zipfile, gzip, time, struct, subprocess, shutil
ROOT=Path(__file__).resolve().parents[1]
CACHE=ROOT/'.cache'/'downloads'; CACHE.mkdir(parents=True,exist_ok=True)
ASSETS=ROOT/'assets'; ASSETS.mkdir(exist_ok=True)
def fetch(url):
    for attempt in range(4):
        try:
            with urllib.request.urlopen(urllib.request.Request(url,headers={'User-Agent':'U-Manga-build/0.1'}),timeout=180) as r:return r.read()
        except Exception:
            if attempt==3:raise
            time.sleep(2**attempt)
def download(url,name):
    dest=CACHE/name
    if not dest.exists():
        print('Downloading',name,flush=True); b=fetch(url); dest.write_bytes(b)
    return dest.read_bytes()
def write(path,data):path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
def runtime():
    folder=ASSETS/'runtime'; folder.mkdir(exist_ok=True)
    url='https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg'
    with zipfile.ZipFile(io.BytesIO(download(url,'ort-directml-1.24.4.zip'))) as z:
        for n in z.namelist():
            if n.startswith('runtimes/win-x64/native/') and n.endswith('.dll'):write(folder/Path(n).name,z.read(n))
            if n.lower().endswith(('license','license.txt','thirdpartynotices.txt')):write(ASSETS/'licenses'/('ort-'+Path(n).name),z.read(n))
    url='https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg'
    with zipfile.ZipFile(io.BytesIO(download(url,'directml-1.15.4.zip'))) as z:
        for n in z.namelist():
            if n.lower().endswith('/x64-win/directml.dll'):write(folder/'DirectML.dll',z.read(n))
            if 'license' in n.lower() and not n.endswith('/'):write(ASSETS/'licenses'/('directml-'+Path(n).name),z.read(n))
    url='https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8066/pdfium-win-x64.tgz'
    with tarfile.open(fileobj=io.BytesIO(download(url,'pdfium-8066-win-x64.tgz')),mode='r:gz') as z:
        for f in z.getmembers():
            if f.isfile() and (f.name.endswith('pdfium.dll') or 'LICENSE' in f.name or f.name.startswith('licenses/')):
                write(folder/'pdfium.dll' if f.name.endswith('.dll') else ASSETS/'licenses'/('pdfium-'+Path(f.name).name),z.extractfile(f).read())
    # Build-time extraction only: official signed Microsoft redist, pinned by SHA-256.
    redist=download('https://aka.ms/vc14/vc_redist.x64.exe','vc_redist.x64.exe')
    expected='843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c'
    if hashlib.sha256(redist).hexdigest()!=expected:raise RuntimeError('Visual C++ redist changed; review version and pin before building')
    seven=shutil.which('7z') or r'C:\Program Files\7-Zip\7z.exe'
    work=ROOT/'.cache'/'redist';work.mkdir(exist_ok=True)
    pos=0;parts=[]
    while True:
        pos=redist.find(b'MSCF',pos)
        if pos<0:break
        size=struct.unpack_from('<I',redist,pos+8)[0]
        if size>36 and pos+size<=len(redist):
            part=work/f'part{len(parts)}.cab';part.write_bytes(redist[pos:pos+size]);parts.append(part);pos+=size
        else:pos+=4
    subprocess.run([seven,'x',str(parts[1]),'-o'+str(work/'payload'),'-y'],check=True,stdout=subprocess.DEVNULL)
    subprocess.run([seven,'x',str(work/'payload'/'a4'),'-o'+str(work/'crt'),'-y'],check=True,stdout=subprocess.DEVNULL)
    for name in ['msvcp140.dll','msvcp140_1.dll','vcruntime140.dll','vcruntime140_1.dll']:
        write(folder/name,(work/'crt'/(name+'_amd64')).read_bytes())
    write(ASSETS/'licenses'/'VisualCpp-License.html',download('https://visualstudio.microsoft.com/license-terms/vs2026-ga-visualcpp-v14-redist-runtime/','VisualCpp-License.html'))
    records=[]
    for p in sorted(folder.glob('*.dll')):
        b=p.read_bytes();write(p.with_suffix('.dll.gz'),gzip.compress(b,compresslevel=9,mtime=0))
        records.append({'name':p.name,'sha256':hashlib.sha256(b).hexdigest(),'bytes':len(b)})
    (ASSETS/'runtime-manifest.json').write_text(json.dumps(records,indent=2))
    print('Runtime bytes',sum(x['bytes'] for x in records),flush=True)
def fonts():
    fonts={
      'NotoSansCJK-Regular.ttc':'https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/OTC/NotoSansCJK-Regular.ttc',
      'NotoSansArabic.ttf':'https://raw.githubusercontent.com/notofonts/arabic/main/fonts/NotoSansArabic/unhinted/variable/NotoSansArabic[wght,wdth].ttf',
      'NotoSansDevanagari.ttf':'https://raw.githubusercontent.com/notofonts/devanagari/main/fonts/NotoSansDevanagari/unhinted/variable/NotoSansDevanagari[wdth,wght].ttf',
    }
    # Google Fonts provides stable individual font assets for complex-script fallback.
    fonts['NotoSansArabic.ttf']='https://raw.githubusercontent.com/google/fonts/main/ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf'
    fonts['NotoSansDevanagari.ttf']='https://raw.githubusercontent.com/google/fonts/main/ofl/notosansdevanagari/NotoSansDevanagari%5Bwdth,wght%5D.ttf'
    for name,url in fonts.items():write(ASSETS/'fonts'/name,download(url,name))
    for family in ['notosansarabic','notosansdevanagari']:
        write(ASSETS/'licenses'/(family+'-OFL.txt'),fetch(f'https://raw.githubusercontent.com/google/fonts/main/ofl/{family}/OFL.txt'))
    write(ASSETS/'licenses'/'NotoCJK-LICENSE.txt',fetch('https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/LICENSE'))
def catalog(selected):
    existing={p['id']:p for p in json.loads((ASSETS/'models.json').read_text(encoding='utf-8'))}
    specs=[('rtdetr_int8','RT-DETR small v4','detector','ogkalu/comic-text-and-bubble-detector',['detector-v4-s_int8.onnx'],[]),
           ('manga_ocr_onnx','Manga OCR Japanese','manga','mayocream/manga-ocr-onnx',['encoder_model.onnx','decoder_model.onnx','vocab.txt','config.json','preprocessor_config.json'],['ja']),
           ('pp_det','PP-OCRv5 text lines','line-detector','PaddlePaddle/PP-OCRv5_mobile_det_onnx',['inference.onnx','inference.yml'],[])]
    packs=[('cjk','PP-OCRv5_mobile_rec',['zh','zh-Hans','zh-Hant','ja']),('latin','latin_PP-OCRv5_mobile_rec',['en','fr','de','es','pt','it','vi','pl','tr','nl','id','ms','ro','sv','da','fi','cs']),('korean','korean_PP-OCRv5_mobile_rec',['ko']),('cyrillic','cyrillic_PP-OCRv5_mobile_rec',['ru','uk','bg','sr']),('arabic','arabic_PP-OCRv5_mobile_rec',['ar','fa','ur']),('devanagari','devanagari_PP-OCRv5_mobile_rec',['hi','mr','ne']),('thai','th_PP-OCRv5_mobile_rec',['th']),('greek','el_PP-OCRv5_mobile_rec',['el']),('tamil','ta_PP-OCRv5_mobile_rec',['ta'])]
    for key,name,langs in packs:specs.append(('pp_'+key,'PP-OCRv5 '+key,'recognizer','PaddlePaddle/'+name+'_onnx',['inference.onnx','inference.yml'],langs))
    records=[]
    for ident,name,kind,repo,names,langs in specs:
        meta=json.loads(fetch('https://huggingface.co/api/models/'+repo+'?blobs=true')); revision=meta['sha'];siblings={f['rfilename']:f for f in meta['siblings']};files=[]
        for n in names:
            m=siblings[n];url=f'https://huggingface.co/{repo}/resolve/{revision}/{n}';h=m.get('lfs',{}).get('sha256');size=m.get('size',m.get('lfs',{}).get('size',0))
            b=None
            if not h or ident in selected:
                b=download(url,ident+'-'+n);actual=hashlib.sha256(b).hexdigest()
                if h and h!=actual:raise RuntimeError('Hash mismatch: '+ident+'/'+n)
                h=actual;size=len(b)
            if ident in selected:write(ASSETS/'models'/ident/n,b)
            files.append({'name':n,'url':url,'sha256':h,'bytes':size})
        records.append({'id':ident,'name':name,'description':existing.get(ident,{}).get('description',name+' local recognition pack.'),'kind':kind,'languages':langs,'license':'Apache-2.0','revision':revision,'distribution':'bundled' if ident=='rtdetr_int8' else 'download','files':files})
        print('Catalog',ident,sum(f['bytes'] for f in files),flush=True)
    records.extend(p for p in existing.values() if p['kind']=='inpainting')
    (ASSETS/'models.json').write_text(json.dumps(records,ensure_ascii=False,indent=2),encoding='utf-8')
def bundled_models():
    # Consume the committed pin; never refresh model revisions during packaging.
    for pack in json.loads((ASSETS/'models.json').read_text(encoding='utf-8')):
        if pack['distribution']!='bundled':continue
        for spec in pack['files']:
            path=ASSETS/'models'/pack['id']/spec['name']
            b=path.read_bytes() if path.is_file() else fetch(spec['url'])
            if len(b)!=spec['bytes'] or hashlib.sha256(b).hexdigest()!=spec['sha256']:
                raise RuntimeError('Bundled model checksum mismatch: '+spec['name'])
            if not path.is_file():write(path,b)
            print('Verified bundled model',pack['id'],len(b),'bytes',flush=True)
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--models',nargs='*',default=[]);p.add_argument('--only',choices=['runtime','fonts','models','bundled-models']);a=p.parse_args()
    if a.only in (None,'runtime'):runtime()
    if a.only in (None,'fonts'):fonts()
    if a.only in (None,'models'):catalog(a.models)

    if a.only=='bundled-models':bundled_models()
