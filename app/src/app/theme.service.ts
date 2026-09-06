import { DOCUMENT } from '@angular/common';
import { effect, inject, Injectable, signal } from '@angular/core';

export type Theme = 'light' | 'dark';

@Injectable({ providedIn: 'root' })
export class ThemeService {
  private readonly document = inject(DOCUMENT);
  readonly theme = signal<Theme>(this.savedTheme());

  constructor() {
    effect(() => {
      const theme = this.theme();
      this.document.documentElement.dataset['theme'] = theme;
      this.document.defaultView?.localStorage?.setItem('teryon-theme', theme);
    });
  }

  set(theme: Theme): void {
    this.theme.set(theme);
  }

  toggle(): void {
    this.theme.update((theme) => (theme === 'dark' ? 'light' : 'dark'));
  }

  private savedTheme(): Theme {
    const saved = this.document.defaultView?.localStorage?.getItem('teryon-theme');
    if (saved === 'light' || saved === 'dark') return saved;
    return this.document.defaultView?.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
}
