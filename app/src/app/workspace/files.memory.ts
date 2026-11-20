import { Injectable } from '@angular/core';
import { BehaviorSubject, distinctUntilChanged, map, type Observable } from 'rxjs';
import { NOTHING_REMEMBERED, type Remembered } from './files.model';

/**
 * The open file and folders of each place the file browser showed.
 *
 * The file browser is removed whenever the person goes back to the thread or
 * to the history, so what they left open has to be kept outside of it to be
 * there when they come back. It is kept for as long as the page is open, and
 * forgotten on a reload.
 *
 * Places are told apart by `placeKey`.
 */
@Injectable({ providedIn: 'root' })
export class FilesMemory {
  private readonly places$ = new BehaviorSubject<ReadonlyMap<string, Remembered>>(new Map());

  /** What is remembered of a place, and every change to it. */
  remembered(place: string): Observable<Remembered> {
    return this.places$.pipe(
      map((places) => places.get(place) ?? NOTHING_REMEMBERED),
      distinctUntilChanged(),
    );
  }

  /** Remembers the file the person opened. */
  choose(place: string, path: string): void {
    this.change(place, (remembered) => ({ ...remembered, chosen: path }));
  }

  /** Remembers that the person opened or closed a folder. */
  toggle(place: string, path: string, open: boolean): void {
    this.change(place, (remembered) => ({ ...remembered, toggled: new Map(remembered.toggled).set(path, open) }));
  }

  private change(place: string, update: (remembered: Remembered) => Remembered): void {
    const places = this.places$.value;
    this.places$.next(new Map(places).set(place, update(places.get(place) ?? NOTHING_REMEMBERED)));
  }
}
