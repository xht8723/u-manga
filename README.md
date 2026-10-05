# U-Manga

**English** | [简体中文](README.zh-CN.md)

[Download for Windows x64](https://github.com/xht8723/u-manga/releases/latest/download/U-Manga.exe) · [Releases](https://github.com/xht8723/u-manga/releases)

U-Manga is a Windows app for reading, translating, and editing manga. Bring your own manga files, choose local models or a translation service, and work from original pages to cleaned, typeset translations in one workspace.

Read on your PC, or host your library and open it from a phone or tablet browser. The desktop app handles storage and processing for both.

## Features

### Library and reading

- Import image folders, PNG/JPEG/WebP images, CBZ/ZIP archives, and PDFs.
- Organize books and chapters, edit metadata and covers, reorder pages, and mark chapters read or unread.
- Use grid, card, or list views with search, sorting, and filters.
- Read in continuous or paged mode with thumbnails, zoom, and an Original / Translated selector.
- Enable **Translate as I read** to request translations while reading.
- Customize Day and Night themes and reader appearance. Reader filters do not change exported images.
- Switch between English and Simplified Chinese instantly, or follow the system language.

### Automatic and manual translation

- Detect text regions locally with the bundled dialogue detector.
- Choose **Local OCR** to recognize text on the PC, then send text to the selected translator.
- Choose **Vision** to let a compatible image service read detected text crops.
- Use Manga OCR or multilingual PP-OCR packs, with CPU and supported DirectML acceleration.
- Translate a page, chapter, or full book. Batch translation offers **Skip already translated pages** and **Replace existing work**.
- Omit selected pages from batch translation without removing them from the book.
- Prepare pages for manual translation when you want to supply the wording yourself.
- Use optional preceding-page original dialogue to help resolve ambiguous sentences.

### Translation services and prompts

- Connect to **Ollama** on this PC or another machine using your own installed models.
- Use **OpenAI-compatible Chat Completions**, **OpenAI Responses**, **Anthropic**, or **Gemini** services.
- Connect to **Google Cloud Translation**, **Microsoft Translator**, **DeepL**, or **Baidu Translate**.
- Choose structured LLM replies or **Simple translation mode**, which translates a page's text using numbered replies and splits large pages.
- Customize text, simple-text, Vision, and glossary prompts per service profile, and inspect sample requests before using them.
- Control the model's Thinking preference where supported.

Available languages and capabilities depend on the selected OCR pack, model, and service. Ollama and its model weights are managed separately from U-Manga.

### Cleanup and lettering

- Remove original text with solid fill or local inpainting using **Manga LaMa**, **Manga AOT**, or **MI-GAN**.
- Adjust OCR and cleanup devices independently, with visible CPU fallback when acceleration is unavailable.
- Render horizontal or vertical text with automatic font fitting, centered text blocks, and aligned vertical column tops.
- Use bundled fallback fonts and installed system fonts.
- Set text color and optional outlines, including outline color and thickness.
- Import a cleaned background, approve solid-color cleanup, or keep the background and add lettering only.

### Region editing

- Select, draw, move, resize, or delete text regions.
- Correct source text and translations, or request OCR and retranslation for an individual region.
- Adjust coordinates, direction, font, size, colors, outlines, and cleanup choices.
- Save or discard changes per region, with Undo/Redo for supported draft edits.
- Review processing details without leaving the page. Failed saves retain the draft.

### Book glossaries

- Keep a separate glossary for each book to maintain consistent names and terminology.
- Search and edit entries, add terms, and delete selected rows.
- Import or export CSV/TSV files, with an import preview and conflict choices.
- Optionally let an LLM detect and save new terms during translation.
- Inspect and edit glossaries from book actions, Editor, and the hosted browser interface.

Glossary use and automatic detection are optional. Support varies by service; DeepL uses a configured hosted glossary ID.

### Background Jobs

- Queue work and follow detection, OCR, translation, glossary, cleanup, and lettering progress.
- Pause, resume, retry, or cancel individual Jobs, or use **Start all / Stop all**.
- Process one page operation per book at a time, with adjustable concurrency across different books.
- Resume interrupted work using saved checkpoints and cached successful results.
- Continue reading and editing while background work runs.

### Phone and tablet access

- Start the built-in server with **Host server** in Library.
- Connect by URL or QR code, then sign in with the shared password.
- Browse books, read original or translated pages, request translation, and manage Jobs.
- Correct region text, delete regions, and edit book glossaries.
- Use compact mobile layouts with browser-local language, theme, and reader choices.

Keep U-Manga open and the PC awake. Hosting uses HTTP on trusted local networks; access away from home requires a separately configured private VPN. Detailed geometry and typography editing, model setup, and library administration stay on the desktop.

### Export and storage

- Export books or chapters as **CBZ**, **PDF**, or **PNG images**.
- Keep original files unchanged, with edits and generated assets stored separately in the library.
- Reuse saved cleanup when eligible text or typography edits only need new lettering.
- Keep service credentials in the operating system's credential store.

Original files remain linked, so keep them accessible. Local OCR and cleanup run on the PC; configured translation services receive the text or image crops required by the selected workflow.

## Getting started

1. Run the portable **U-Manga.exe** on Windows x64. Microsoft WebView2 is required.
2. Choose your library, interface language, translation languages, and processing setup.
3. Download the optional OCR and cleanup packs you select, and configure a translation service if you want automatic translation. Dialogue detection is already bundled.
4. Add a book, organize its chapters, and open Reader or Editor.
5. Translate, review the result, make corrections, and export when ready.

The published app does not require Python, Node.js, or CUDA. Translation services may require an account or API key and may charge for requests. Hosting is optional and starts only when requested.

## Building from source

U-Manga uses **Tauri 2**, **Rust**, **Svelte 5**, and **TypeScript**.

On Windows x64, install Rust with the MSVC toolchain, Visual Studio C++ build tools and Windows SDK, Node.js, pnpm, and Python 3.10 or newer. Then run from the repository root:

```powershell
.\scripts\build.ps1
```

If Python is not on PATH, pass `-PythonExecutable <path-to-python.exe>`. The script restores pinned dependencies, runs the frontend and native checks, builds and publishes `release/U-Manga.exe`, and removes disposable build output. Add `-KeepBuildCache` to retain it for development.

Bundled build assets are included in this repository. Recreating them with `-PrepareAssets` requires network access and 7-Zip; optional application model downloads are not required to compile.
