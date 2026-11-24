import {
  AfterViewInit,
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  EventEmitter,
  inject,
  Input,
  Output,
} from '@angular/core';
import { BehaviorSubject, combineLatest, map } from 'rxjs';
import { filteredSections, SHARED_KEYS, type ShortcutSection } from './shortcuts.model';

/** The searchable list of shortcuts for the workspace screen underneath it. */
@Component({
  selector: 'app-shortcuts',
  templateUrl: './shortcuts.component.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
  standalone: false,
})
export class ShortcutsComponent implements AfterViewInit {
  private readonly host: ElementRef<HTMLElement> = inject(ElementRef);
  private readonly sections$ = new BehaviorSubject<readonly ShortcutSection[]>([]);
  private readonly typed$ = new BehaviorSubject('');

  @Input({ required: true }) set sections(sections: readonly ShortcutSection[]) {
    this.sections$.next(sections);
  }

  @Output() readonly closed = new EventEmitter<void>();

  readonly sharedKeys = SHARED_KEYS;
  readonly filtered$ = combineLatest([this.sections$, this.typed$]).pipe(
    map(([sections, typed]) => filteredSections(sections, typed)),
  );

  ngAfterViewInit(): void {
    // A touch screen can show the shortcuts without raising the keyboard.
    const first = matchMedia('(pointer: coarse)').matches ? '[role="dialog"]' : 'input';
    this.host.nativeElement.querySelector<HTMLElement>(first)?.focus();
  }

  search(event: Event): void {
    this.typed$.next((event.target as HTMLInputElement).value);
  }
}
