import { AfterViewInit, Component, ElementRef, OnDestroy, effect, input, output, viewChild } from '@angular/core';
// Full editor (find/replace, folding, multi-cursor, …). Its CSS imports are
// not injected by the Angular bundler; the stylesheet comes from angular.json
// (monaco-editor/min/vs/editor/editor.main.css) instead.
import * as monaco from 'monaco-editor';

// No web worker is bundled: Monaco then runs its (for Nix unused) language
// services on the main thread, which is fine for configuration-sized files.
// A no-op worker factory keeps it from logging an error on every page.
declare global {
  interface Window { MonacoEnvironment?: monaco.Environment }
}
if (!window.MonacoEnvironment) {
  window.MonacoEnvironment = {
    getWorker: () => new Worker(URL.createObjectURL(new Blob(['self.onmessage = () => {};'], { type: 'text/javascript' }))),
  };
}

let nixRegistered = false;
function registerNix() {
  if (nixRegistered) return;
  nixRegistered = true;
  monaco.languages.register({ id: 'nix', extensions: ['.nix'], aliases: ['Nix'] });
  monaco.languages.setLanguageConfiguration('nix', {
    comments: { lineComment: '#', blockComment: ['/*', '*/'] },
    brackets: [['{', '}'], ['[', ']'], ['(', ')']],
    autoClosingPairs: [
      { open: '{', close: '}' }, { open: '[', close: ']' }, { open: '(', close: ')' },
      { open: '"', close: '"', notIn: ['string'] }, { open: "''", close: "''", notIn: ['string'] },
    ],
    surroundingPairs: [{ open: '{', close: '}' }, { open: '[', close: ']' }, { open: '(', close: ')' }, { open: '"', close: '"' }],
    indentationRules: { increaseIndentPattern: /[{[(]\s*$/, decreaseIndentPattern: /^\s*[}\])]/ },
  });
  monaco.languages.setMonarchTokensProvider('nix', {
    defaultToken: '',
    keywords: ['let', 'in', 'with', 'rec', 'inherit', 'if', 'then', 'else', 'assert', 'or'],
    constants: ['true', 'false', 'null'],
    builtins: ['import', 'builtins', 'map', 'toString', 'throw', 'abort', 'derivation', 'fetchTarball', 'removeAttrs', 'baseNameOf', 'dirOf', 'isNull', 'config', 'pkgs', 'lib', 'vars', 'self', 'inputs'],
    operators: ['=', '==', '!=', '<', '<=', '>', '>=', '++', '+', '-', '*', '/', '//', '?', '->', '&&', '||', '!', ':', '@'],
    symbols: /[=><!?:&|+\-*/^%@]+/,
    tokenizer: {
      root: [
        [/[a-zA-Z_][\w'-]*(?=\s*=)/, 'variable.name'],
        [/[a-zA-Z_][\w'-]*(?=\.[a-zA-Z_])/, 'variable.name'],
        [/[a-zA-Z_][\w'-]*/, { cases: { '@keywords': 'keyword', '@constants': 'constant', '@builtins': 'type.identifier', '@default': 'identifier' } }],
        [/\.{0,2}\/[\w./+-]*/, 'string.path'],
        [/~\/[\w./+-]*/, 'string.path'],
        [/<[a-zA-Z][\w./-]*>/, 'string.path'],
        [/\d+\.\d+([eE][-+]?\d+)?/, 'number.float'],
        [/\d+/, 'number'],
        [/#.*$/, 'comment'],
        [/\/\*/, 'comment', '@blockComment'],
        [/''/, { token: 'string.quote', next: '@indentedString' }],
        [/"/, { token: 'string.quote', next: '@string' }],
        [/[{}()[\]]/, '@brackets'],
        [/@symbols/, { cases: { '@operators': 'operator', '@default': '' } }],
        [/[;,.]/, 'delimiter'],
      ],
      blockComment: [[/[^*/]+/, 'comment'], [/\*\//, 'comment', '@pop'], [/[*/]/, 'comment']],
      string: [
        [/\$\{/, { token: 'delimiter.bracket', next: '@interp' }],
        [/[^\\"$]+/, 'string'],
        [/\\./, 'string.escape'],
        [/\$(?!\{)/, 'string'],
        [/"/, { token: 'string.quote', next: '@pop' }],
      ],
      indentedString: [
        [/\$\{/, { token: 'delimiter.bracket', next: '@interp' }],
        [/[^'$]+/, 'string'],
        [/'''/, 'string.escape'],
        [/''\$/, 'string.escape'],
        [/''/, { token: 'string.quote', next: '@pop' }],
        [/['$]/, 'string'],
      ],
      interp: [
        [/\}/, { token: 'delimiter.bracket', next: '@pop' }],
        [/\{/, { token: 'delimiter.bracket', next: '@interpNested' }],
        { include: 'root' },
      ],
      interpNested: [
        [/\}/, { token: 'delimiter.bracket', next: '@pop' }],
        [/\{/, { token: 'delimiter.bracket', next: '@interpNested' }],
        { include: 'root' },
      ],
    },
  } as monaco.languages.IMonarchLanguage);
}

/**
 * Monaco-based code editor (Nix highlighting, bracket matching, find/replace,
 * multi-cursor, …). `value` is the model; `valueChange` fires on every edit.
 */
@Component({
  selector: 'app-code-editor',
  template: `<div #host class="host" [style.height]="height()"></div>`,
  styles: [`
    :host { display: block; }
    .host { width: 100%; border: 1px solid var(--mat-sys-outline-variant); border-radius: 6px; overflow: hidden; }
  `],
})
export class CodeEditorComponent implements AfterViewInit, OnDestroy {
  value = input<string>('');
  language = input<string>('nix');
  height = input<string>('60vh');
  readOnly = input<boolean>(false);
  /** Problems to underline (1-based lines/columns). */
  markers = input<{ line: number; column: number; endLine?: number; endColumn?: number; message: string }[]>([]);
  valueChange = output<string>();
  /** Ctrl+S / Cmd+S pressed inside the editor. */
  save = output<void>();
  /** The editor exists (safe to call revealLine). */
  ready = output<void>();

  private host = viewChild.required<ElementRef<HTMLDivElement>>('host');
  private editor?: monaco.editor.IStandaloneCodeEditor;
  private suppress = false;

  constructor() {
    effect(() => {
      const ms = this.markers();
      const model = this.editor?.getModel();
      if (!model) return;
      monaco.editor.setModelMarkers(model, 'nix', ms.map(m => ({
        startLineNumber: m.line, startColumn: m.column, endLineNumber: m.endLine ?? m.line, endColumn: m.endColumn ?? m.column + 1,
        message: m.message, severity: monaco.MarkerSeverity.Error,
      })));
    });
    // push external value changes into the editor without clobbering the caret on our own edits
    effect(() => {
      const v = this.value();
      if (this.editor && !this.suppress && this.editor.getValue() !== v) {
        this.editor.setValue(v);
      }
    });
  }

  ngAfterViewInit() {
    registerNix();
    this.editor = monaco.editor.create(this.host().nativeElement, {
      value: this.value(),
      language: this.language(),
      theme: 'vs-dark',
      readOnly: this.readOnly(),
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      lineNumbers: 'on',
      scrollBeyondLastLine: false,
      wordWrap: 'off',
      tabSize: 2,
      insertSpaces: true,
      renderWhitespace: 'none',
      bracketPairColorization: { enabled: true },
      folding: true,
      fixedOverflowWidgets: true,
    });
    (this.host().nativeElement as HTMLDivElement & { __monaco?: monaco.editor.IStandaloneCodeEditor }).__monaco = this.editor;
    // Ctrl+S saves instead of opening the browser's "save page" dialog
    this.editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => this.save.emit());
    this.ready.emit();
    // markers set before the editor existed
    const ms = this.markers();
    if (ms.length) monaco.editor.setModelMarkers(this.editor.getModel()!, 'nix', ms.map(m => ({ startLineNumber: m.line, startColumn: m.column, endLineNumber: m.endLine ?? m.line, endColumn: m.endColumn ?? m.column + 1, message: m.message, severity: monaco.MarkerSeverity.Error })));
    this.editor.onDidChangeModelContent(() => {
      this.suppress = true;
      try { this.valueChange.emit(this.editor!.getValue()); } finally { this.suppress = false; }
    });
  }

  /** Insert text at the caret (replacing a selection) and keep focus. */
  insertAtCursor(text: string) {
    const ed = this.editor;
    if (!ed) return;
    const sel = ed.getSelection();
    const range = sel ?? new monaco.Range(1, 1, 1, 1);
    ed.executeEdits('nixos-manager', [{ range, text, forceMoveMarkers: true }]);
    ed.focus();
  }

  focus() { this.editor?.focus(); }

  /** Scroll to a 1-based line, put the caret there and highlight it briefly. */
  revealLine(line: number) {
    const ed = this.editor;
    if (!ed) return;
    ed.revealLineInCenter(line);
    ed.setPosition({ lineNumber: line, column: 1 });
    ed.focus();
  }

  ngOnDestroy() {
    this.editor?.dispose();
  }
}
