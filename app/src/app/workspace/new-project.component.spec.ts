import { CommonModule } from '@angular/common';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { NgModule } from '@angular/core';
import { Subject } from 'rxjs';

import type { DirectoryListing } from './new-project.model';
import { type NewProject, NewProjectComponent } from './new-project.component';
import { WorkspaceApi } from './workspace.api';

/** The form, with the pipe its template uses, and nothing else from the workspace. */
@NgModule({
  declarations: [NewProjectComponent],
  imports: [CommonModule],
})
class NewProjectTestModule {}

/** Listings the form asks for, answered when the test says so. */
class DirectoryDouble {
  readonly asked: string[] = [];
  private readonly pending = new Map<string, Subject<DirectoryListing>>();

  directories(path: string) {
    this.asked.push(path);
    const reply = new Subject<DirectoryListing>();
    this.pending.set(path, reply);
    return reply.asObservable();
  }

  answer(path: string, listing: DirectoryListing): void {
    const reply = this.pending.get(path);
    if (!reply) throw new Error(`nothing was asked for ${JSON.stringify(path)}`);
    reply.next(listing);
  }
}

const home: DirectoryListing = {
  directory: '/home/you',
  parent: '/',
  children: ['code', 'projects'],
  remainder: '',
};

describe('NewProjectComponent', () => {
  let fixture: ComponentFixture<NewProjectComponent>;
  let directories: DirectoryDouble;
  let added: NewProject[];

  beforeEach(async () => {
    directories = new DirectoryDouble();
    added = [];
    await TestBed.configureTestingModule({
      imports: [NewProjectTestModule],
      providers: [{ provide: WorkspaceApi, useValue: directories }],
    }).compileComponents();
    vi.useFakeTimers();
    fixture = TestBed.createComponent(NewProjectComponent);
    fixture.componentInstance.added.subscribe((project) => added.push(project));
    fixture.detectChanges();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function field(): HTMLInputElement {
    const input = fixture.nativeElement.querySelector('[aria-label="Project directory"]');
    if (!(input instanceof HTMLInputElement)) throw new Error('no directory field');
    return input;
  }

  function type(path: string): void {
    const input = field();
    input.value = path;
    input.dispatchEvent(new Event('input'));
    fixture.detectChanges();
  }

  async function settle(): Promise<void> {
    await vi.advanceTimersByTimeAsync(40);
    fixture.detectChanges();
  }

  it('does not ask again while a name is still being typed', async () => {
    await settle();
    directories.answer('', home);
    fixture.detectChanges();
    await settle();

    type('/home/you/c');
    await settle();
    type('/home/you/co');
    await settle();

    expect(directories.asked).toEqual(['']);
    expect(field().value).toBe('/home/you/co');
    expect(fixture.nativeElement.textContent).toContain('code');
    expect(fixture.nativeElement.textContent).not.toContain('projects');
  });

  it('asks for a directory stepped into from the list', async () => {
    await settle();
    directories.answer('', home);
    fixture.detectChanges();
    await settle();

    type('/home/you/co');
    await settle();
    field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle();

    expect(directories.asked).toEqual(['', '/home/you/code']);
    expect(field().value).toBe('/home/you/code');
  });

  it('keeps letters typed after ~ when the answer for ~ arrives late', async () => {
    type('~');
    await settle();
    expect(directories.asked).toEqual(['~']);

    type('~/c');
    directories.answer('~', home);
    fixture.detectChanges();

    expect(field().value).toBe('~/c');
    expect(fixture.nativeElement.querySelector('[aria-label="Project name"]')).toHaveProperty('value', '');

    await settle();
    expect(field().value).toBe('~/c');
    expect(directories.asked).toEqual(['~']);
    expect(fixture.nativeElement.textContent).toContain('code');
  });

  it('rewrites ~ once that is the whole path', async () => {
    type('~');
    await settle();
    directories.answer('~', home);
    fixture.detectChanges();

    expect(field().value).toBe('/home/you');
    expect(fixture.nativeElement.querySelector('[aria-label="Project name"]')).toHaveProperty('value', 'you');
  });

  it('does not add the project when enter finds no directory', async () => {
    await settle();
    directories.answer('', home);
    fixture.detectChanges();
    await settle();

    type('/home/you/zzz');
    await settle();
    const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    field().dispatchEvent(event);
    fixture.detectChanges();

    expect(event.defaultPrevented).toBe(true);
    expect(added).toEqual([]);
    expect(field().value).toBe('/home/you/zzz');
    expect(fixture.nativeElement.textContent).toContain('No directory matches.');
  });
});
