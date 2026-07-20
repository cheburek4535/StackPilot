<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { EditorView, basicSetup } from "codemirror";
  import { EditorState } from "@codemirror/state";
  import { javascript } from "@codemirror/lang-javascript";
  import { python } from "@codemirror/lang-python";
  import { rust } from "@codemirror/lang-rust";
  import { json } from "@codemirror/lang-json";
  import { html } from "@codemirror/lang-html";
  import { css } from "@codemirror/lang-css";
  import { oneDark } from "@codemirror/theme-one-dark";
  import { keymap } from "@codemirror/view";

  let { value = "", language = "plaintext", readonly = false, onchange, onsave }: {
    value?: string;
    language?: string;
    readonly?: boolean;
    onchange?: (val: string) => void;
    onsave?: (val: string) => void;
  } = $props();

  let container: HTMLDivElement;
  let view: EditorView;

  function getExtensions() {
    const exts = [basicSetup];
    if (readonly) exts.push(EditorView.editable.of(false));
    const lang = language.toLowerCase();
    if (lang === "javascript" || lang === "js" || lang === "jsx" || lang === "typescript" || lang === "ts" || lang === "tsx") {
      exts.push(javascript());
    } else if (lang === "python" || lang === "py") {
      exts.push(python());
    } else if (lang === "rust" || lang === "rs") {
      exts.push(rust());
    } else if (lang === "json") {
      exts.push(json());
    } else if (lang === "html" || lang === "htm") {
      exts.push(html());
    } else if (lang === "css") {
      exts.push(css());
    }
    if (matchMedia("(prefers-color-scheme: dark)").matches) {
      exts.push(oneDark);
    }
    if (onsave) {
      exts.push(keymap.of([{
        key: "Mod-s",
        run: () => {
          onsave(view.state.doc.toString());
          return true;
        },
      }]));
    }
    return exts;
  }

  onMount(() => {
    const state = EditorState.create({
      doc: value,
      extensions: [
        ...getExtensions(),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && onchange) {
            onchange(update.state.doc.toString());
          }
        }),
      ],
    });
    view = new EditorView({ state, parent: container });
  });

  function setValue(val: string) {
    if (view) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: val },
      });
    }
  }

  $effect(() => {
    if (view && value !== view.state.doc.toString()) {
      setValue(value);
    }
  });

  onDestroy(() => {
    view?.destroy();
  });
</script>

<div bind:this={container} class="cm-container"></div>

<style>
  .cm-container {
    height: 100%;
    overflow: hidden;
    border-radius: 6px;
    border: 1px solid #e0e0e0;
  }
  .cm-container :global(.cm-editor) {
    height: 100%;
  }
  .cm-container :global(.cm-scroller) {
    overflow: auto;
  }

  @media (prefers-color-scheme: dark) {
    .cm-container {
      border-color: #333;
    }
  }
</style>
