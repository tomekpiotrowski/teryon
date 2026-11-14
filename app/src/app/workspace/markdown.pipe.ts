import { Pipe, type PipeTransform } from '@angular/core';
import { renderMarkdown } from './markdown';

/**
 * Renders markdown as HTML, for binding to `innerHTML`.
 *
 * A pipe rather than a field on each thread line, so the HTML is rebuilt only
 * when the text itself changes and not every time the thread is redrawn.
 *
 * The result goes through Angular's own cleaning of bound HTML as well, which
 * is a second guard behind the rules in `renderMarkdown`.
 */
@Pipe({ name: 'markdown', standalone: false })
export class MarkdownPipe implements PipeTransform {
  transform(text: string): string {
    return renderMarkdown(text);
  }
}
