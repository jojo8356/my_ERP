#!/usr/bin/env python3
"""Convertit CDC-ERP-CLI.md en CDC.html : un artefact de lecture autonome.

Usage : python3 scripts/build_cdc_html.py
Requiert le paquet `markdown` (pip install markdown) — uniquement au moment de générer.
Le HTML produit est 100 % autonome (aucune ressource externe), lisible hors-ligne.
"""
import pathlib
import re
import sys

try:
    import markdown
except ImportError:
    sys.exit("Paquet manquant : pip install markdown")

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "CDC-ERP-CLI.md"
OUT = ROOT / "CDC.html"

CSS = """
:root{--bg:#f4f1ea;--paper:#fffdf8;--ink:#1c1917;--muted:#78716c;--line:#e7e0d2;--accent:#92400e;--accent2:#b45309;--soft:#fdf3e3;--code:#292524;--rad:14px}
body.dark{--bg:#141210;--paper:#1e1b18;--ink:#e7e5e4;--muted:#a8a29e;--line:#3a352f;--accent:#f59e0b;--accent2:#fbbf24;--soft:#2a2118;--code:#0c0a09}
*{box-sizing:border-box} html{scroll-behavior:smooth}
body{margin:0;background:var(--bg);color:var(--ink);font-family:Georgia,'Iowan Old Style','Times New Roman',serif;line-height:1.75;font-size:18px;transition:background .3s,color .3s}
#progress{position:fixed;top:0;left:0;height:4px;background:linear-gradient(90deg,var(--accent),var(--accent2));width:0;z-index:50}
header{position:sticky;top:0;z-index:40;background:color-mix(in srgb,var(--paper) 88%, transparent);backdrop-filter:blur(8px);border-bottom:1px solid var(--line)}
.hwrap{max-width:1180px;margin:0 auto;padding:10px 20px;display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.hwrap .title{font-family:system-ui,sans-serif;font-weight:800;font-size:15px;margin-right:auto}
button{cursor:pointer;font-family:system-ui,sans-serif;font-size:13.5px;font-weight:600;border:1px solid var(--line);background:var(--paper);color:var(--ink);border-radius:999px;padding:7px 13px}
button:hover{border-color:var(--accent);color:var(--accent)}
input[type=search]{font-family:system-ui,sans-serif;font-size:14px;padding:8px 13px;border:1px solid var(--line);border-radius:999px;background:var(--paper);color:var(--ink);min-width:190px}
.layout{max-width:1180px;margin:0 auto;display:grid;grid-template-columns:250px minmax(0,1fr);gap:34px;padding:28px 20px 80px}
aside{position:sticky;top:70px;align-self:start;max-height:calc(100vh - 90px);overflow:auto;font-family:system-ui,sans-serif;font-size:13.5px}
aside h3{font-size:12px;text-transform:uppercase;letter-spacing:.08em;color:var(--muted);margin:0 0 8px}
aside ul{list-style:none;margin:0;padding:0} aside li{margin:2px 0} aside li li{margin-left:14px;font-size:12.5px}
aside a{color:var(--ink);text-decoration:none;display:block;padding:3px 8px;border-radius:7px}
aside a:hover,aside a.active{background:var(--soft);color:var(--accent)}
article{background:var(--paper);border:1px solid var(--line);border-radius:var(--rad);padding:clamp(22px,4.5vw,52px);box-shadow:0 2px 18px rgba(60,40,10,.05);max-width:820px}
article h1{font-size:clamp(26px,4vw,38px);line-height:1.25;margin:.1em 0 .4em}
article h2{font-size:26px;margin:1.7em 0 .5em;padding-bottom:.25em;border-bottom:2px solid var(--soft);scroll-margin-top:80px}
article h3{font-size:20px;margin:1.4em 0 .4em;scroll-margin-top:80px} article h4{scroll-margin-top:80px}
article p,article li{color:var(--ink)}
article a{color:var(--accent)}
.toc-inline{font-family:system-ui,sans-serif;font-size:15.5px;line-height:1.5}
.toc-inline ol{margin:.4em 0;padding-left:0;list-style:none}
.toc-inline li{margin:5px 0}
.toc-inline a{display:block;text-decoration:none;color:var(--ink);background:var(--soft);border:1px solid var(--line);border-radius:9px;padding:8px 13px;transition:.15s}
.toc-inline a:hover{border-color:var(--accent);color:var(--accent);transform:translateX(3px)}
aside a{cursor:pointer;pointer-events:auto}
article table{border-collapse:collapse;width:100%;font-size:14.5px;font-family:system-ui,sans-serif;line-height:1.5;margin:1em 0;display:block;overflow-x:auto}
th,td{border:1px solid var(--line);padding:8px 10px;text-align:left;vertical-align:top} th{background:var(--soft)}
code{font-family:ui-monospace,Menlo,Consolas,monospace;font-size:.86em;background:var(--soft);padding:.1em .35em;border-radius:5px}
pre{background:var(--code);color:#e7e5e4;padding:16px;border-radius:10px;overflow-x:auto;line-height:1.5}
pre code{background:none;color:inherit;padding:0}
blockquote{border-left:4px solid var(--accent);background:var(--soft);margin:1.2em 0;padding:12px 18px;border-radius:0 10px 10px 0}
hr{border:none;border-top:1px solid var(--line);margin:2.2em 0}
mark{background:#fde68a;color:#111;border-radius:3px;padding:0 2px} body.dark mark{background:#92600a;color:#fff}
#toTop{position:fixed;right:18px;bottom:18px;border-radius:50%;width:46px;height:46px;font-size:19px;box-shadow:0 4px 14px rgba(0,0,0,.18)}
@media(max-width:860px){.layout{grid-template-columns:1fr}aside{position:static;max-height:none;order:2}article{order:1}}
@media print {#progress,header,#toTop,aside{display:none!important} body{background:#fff;font-size:12pt} article{box-shadow:none;border:none;max-width:none} }
"""

JS = """
const $=s=>document.querySelector(s), $$=s=>[...document.querySelectorAll(s)];
// Barre de progression de lecture
addEventListener('scroll',()=>{const h=document.documentElement;const p=h.scrollTop/(h.scrollHeight-h.clientHeight)*100;$('#progress').style.width=p+'%';
 // section active dans le sommaire
 let cur='';$$('article h2,article h3').forEach(x=>{if(x.getBoundingClientRect().top<120)cur=x.id});
 $$('aside a').forEach(a=>a.classList.toggle('active',a.getAttribute('href')==='#'+cur));},{passive:true});
// Sommaire cliquable : défilement doux + décalage sous la barre fixe, pour TOUS les liens d'ancre
$$('a[href^="#"]').forEach(a=>a.addEventListener('click',e=>{const t=document.getElementById(a.getAttribute('href').slice(1));if(t){e.preventDefault();t.scrollIntoView({behavior:'smooth',block:'start'});history.replaceState(null,'',a.getAttribute('href'))}}));
// Mode sombre / clair (mémorisé)
const dark=localStorage.getItem('cdc_dark')==='1'; if(dark)document.body.classList.add('dark');
function toggleDark(){document.body.classList.toggle('dark');localStorage.setItem('cdc_dark',document.body.classList.contains('dark')?'1':'0')}
// Taille du texte (mémorisée)
let fs=parseInt(localStorage.getItem('cdc_fs')||'18'); function setFs(d){fs=Math.min(23,Math.max(14,fs+d));document.body.style.fontSize=fs+'px';localStorage.setItem('cdc_fs',fs)} setFs(0);
// Recherche : surligne les occurrences, Entrée = suivante
let marks=[],mi=0;
function search(q){marks.forEach(m=>{const p=m.parentNode;p.replaceChild(document.createTextNode(m.textContent),m);p.normalize()});marks=[];mi=0;
 if(!q||q.length<3){$('#count').textContent='';return}
 const walker=document.createTreeWalker($('article'),NodeFilter.SHOW_TEXT,{acceptNode:n=>n.parentNode.closest('pre,script,style')||!n.nodeValue.toLowerCase().includes(q.toLowerCase())?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT});
 const nodes=[];while(walker.nextNode())nodes.push(walker.currentNode);
 nodes.forEach(n=>{const i=n.nodeValue.toLowerCase().indexOf(q.toLowerCase());const r=document.createRange();r.setStart(n,i);r.setEnd(n,i+q.length);const m=document.createElement('mark');r.surroundContents(m);marks.push(m)});
 $('#count').textContent=marks.length?marks.length+' résultat(s) — Entrée pour naviguer':'aucun résultat'; if(marks.length)marks[0].scrollIntoView({block:'center'})}
document.addEventListener('keydown',e=>{if(e.key==='Enter'&&document.activeElement.id==='q'&&marks.length){e.preventDefault();mi=(mi+1)%marks.length;marks[mi].scrollIntoView({block:'center',behavior:'smooth'})}});
"""

def main() -> None:
    md_text = SRC.read_text(encoding="utf-8")
    md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"])
    body = md.convert(md_text)
    toc = md.toc
    # Table des matières DANS l'article : le Markdown la contient en texte brut
    # (liste numérotée non cliquable). On la régénère à partir des vrais titres
    # H2 du document, avec leurs ancres réelles -> 100 % cliquable, toujours à jour.
    h2s = [
        (hid, re.sub(r"<[^>]+>", "", label).strip())
        for hid, label in re.findall(r'<h2 id="([^"]+)">(.*?)</h2>', body)
        if hid != "table-des-matieres"
    ]
    inline_toc = (
        '<nav class="toc-inline"><ol>'
        + "".join(f'<li><a href="#{hid}">{label}</a></li>' for hid, label in h2s)
        + "</ol></nav>"
    )
    body, n = re.subn(
        r'(<h2 id="table-des-matieres">.*?</h2>).*?(?=<hr\s*/?>|\s*<h2 )',
        lambda m: m.group(1) + "\n" + inline_toc + "\n",
        body,
        flags=re.S,
    )
    if n == 0:
        print("⚠️  Table des matières intégrée non trouvée : régénération sans remplacement")
    # Le premier H1 du document sert d'en-tête de page : on évite le doublon avec la barre.
    html = f"""<!DOCTYPE html>
<html lang="fr">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Cahier des charges — ERP CLI Rust (lecture)</title>
<style>{CSS}</style>
</head>
<body>
<div id="progress"></div>
<header><div class="hwrap">
  <span class="title">📄 CDC — ERP CLI Rust · v1.1</span>
  <input type="search" id="q" placeholder="🔎 Rechercher dans le CDC…" oninput="search(this.value.trim())" title="Rechercher (Entrée = résultat suivant)">
  <span id="count" style="font-family:system-ui;font-size:13px"></span>
  <button onclick="setFs(-1)" title="Réduire le texte">A−</button>
  <button onclick="setFs(1)" title="Agrandir le texte">A+</button>
  <button onclick="toggleDark()" title="Mode sombre / clair">🌙 / ☀️</button>
  <button onclick="window.print()" title="Imprimer ou enregistrer en PDF">🖨️ PDF</button>
  <a href="CDC-ERP-CLI.md" download style="font-family:system-ui;font-size:13.5px;font-weight:600;color:inherit;text-decoration:none;border:1px solid var(--line);border-radius:999px;padding:7px 13px">⬇️ .md</a>
</div></header>
<div class="layout">
  <aside><h3>Sommaire</h3>{toc}</aside>
  <article>{body}
  <hr><p style="font-family:system-ui;font-size:13px;color:var(--muted)">Version générée depuis <code>CDC-ERP-CLI.md</code> — vos choix : <code>decisions/mes-choix.json</code>. Prochaine action : votre <strong>GO J0</strong> (ou vos corrections).</p></article>
</div>
<button id="toTop" onclick="scrollTo({{top:0,behavior:'smooth'}})" title="Retour en haut">↑</button>
<script>{JS}</script>
</body>
</html>"""
    OUT.write_text(html, encoding="utf-8")
    print(f"{OUT.name} généré ({len(html)//1024} Ko) depuis {SRC.name}")


if __name__ == "__main__":
    main()
