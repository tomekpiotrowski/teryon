import { CommonModule } from '@angular/common';
import { TestBed } from '@angular/core/testing';
import { AgentChooserComponent } from './agent-chooser.component';
import { modelOptionForTest as model } from './model.testing';
import { choosing, type Choice, FIRST_NEW_THREADS, NO_MODELS } from './workspace.model';

const OPUS = model('claude-opus-5-5', 'Opus 5.5', ['low', 'medium', 'xhigh', 'max'], 'medium', true);
const SONNET = model('claude-sonnet-5-5', 'Sonnet 5.5', ['high'], 'high');
const ORCHESTRATOR = model('gemini-pro-orchestrator-neon', 'Orchestrator (Neon)');
const CATALOG = { ...NO_MODELS, claude: [OPUS, SONNET], gemini: [ORCHESTRATOR] };

/** Opens a chooser using the same inputs its parent components bind. */
function create() {
  const fixture = TestBed.createComponent(AgentChooserComponent);
  const host: HTMLElement = fixture.nativeElement;
  const emitted: Choice[] = [];
  fixture.componentInstance.chosen.subscribe((choice) => emitted.push(choice));
  const input = (name: string, value: unknown) => {
    fixture.componentRef.setInput(name, value);
    fixture.detectChanges();
  };
  input('installed', ['claude', 'codex', 'gemini']);
  input('models', CATALOG);
  input('choice', { agent: 'claude', model: null, effort: null });
  const trigger = host.querySelector<HTMLButtonElement>('.agent-trigger')!;
  const options = () => Array.from(host.querySelectorAll<HTMLButtonElement>('[role="option"]'));
  const option = (name: string) => options().find((row) => row.firstElementChild?.textContent === name)!;
  const level = () => host.querySelector('[role="listbox"]')?.getAttribute('aria-label');
  const click = (button: HTMLButtonElement) => {
    button.click();
    fixture.detectChanges();
  };
  const key = (name: string) => {
    document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: name, bubbles: true, cancelable: true }));
    fixture.detectChanges();
  };
  return { fixture, input, host, emitted, trigger, options, option, level, click, key };
}

describe('AgentChooserComponent', () => {
  beforeEach(() => TestBed.configureTestingModule({ imports: [CommonModule], declarations: [AgentChooserComponent] }));

  it('drills into versioned models and canonical efforts and emits the chosen values', () => {
    const chooser = create();
    chooser.click(chooser.trigger);
    expect(chooser.level()).toBe('Agent');
    chooser.click(chooser.option('Claude'));
    expect(chooser.level()).toBe('Model');
    expect(chooser.options().map((row) => row.firstElementChild?.textContent)).toEqual(['Opus 5.5', 'Sonnet 5.5']);
    expect(chooser.option('Opus 5.5').getAttribute('aria-selected')).toBe('true');

    chooser.click(chooser.option('Opus 5.5'));
    expect(chooser.level()).toBe('Effort');
    expect(chooser.options().map((row) => row.textContent?.trim())).toEqual(['Low', 'Medium', 'Extra high', 'Max']);
    expect(chooser.option('Medium').getAttribute('aria-selected')).toBe('true');
    chooser.click(chooser.option('Extra high'));
    expect(chooser.emitted).toEqual([{ agent: 'claude', model: OPUS.id, effort: 'xhigh' }]);
    expect(chooser.level()).toBeUndefined();
    expect(document.activeElement).toBe(chooser.trigger);
  });

  it('wraps keyboard focus, drills right, goes back with left and Backspace, and dismisses on Escape', async () => {
    const chooser = create();
    chooser.trigger.focus();
    chooser.key('ArrowDown');
    await chooser.fixture.whenStable();
    expect(document.activeElement).toBe(chooser.option('Claude'));
    chooser.key('ArrowUp');
    expect(document.activeElement).toBe(chooser.option('Gemini'));
    chooser.key('ArrowDown');
    chooser.key('ArrowRight');
    await chooser.fixture.whenStable();
    expect(document.activeElement).toBe(chooser.option('Opus 5.5'));
    chooser.key('ArrowRight');
    await chooser.fixture.whenStable();
    expect(document.activeElement).toBe(chooser.option('Medium'));
    chooser.key('ArrowLeft');
    await chooser.fixture.whenStable();
    expect(document.activeElement).toBe(chooser.option('Opus 5.5'));
    chooser.key('Backspace');
    await chooser.fixture.whenStable();
    expect(document.activeElement).toBe(chooser.option('Claude'));
    chooser.key('Escape');
    expect(document.activeElement).toBe(chooser.trigger);
    expect(chooser.emitted).toEqual([]);
  });

  it('completes providers with no catalog and models with no efforts immediately', () => {
    const chooser = create();
    chooser.click(chooser.trigger);
    chooser.click(chooser.option('Codex'));
    expect(chooser.level()).toBeUndefined();
    chooser.click(chooser.trigger);
    chooser.click(chooser.option('Gemini'));
    chooser.click(chooser.option('Orchestrator (Neon)'));
    expect(chooser.level()).toBeUndefined();
    expect(chooser.emitted).toEqual([
      { agent: 'codex', model: null, effort: null },
      { agent: 'gemini', model: ORCHESTRATOR.id, effort: null },
    ]);
  });

  it('filters allowed providers, supports both placements and dismisses on an outside click', () => {
    const chooser = create();
    chooser.input('allowedAgents', ['grok', 'gemini']);
    expect(chooser.host.querySelector('.agent-chooser.up')).toBeNull();
    chooser.input('placement', 'up');
    expect(chooser.host.querySelector('.agent-chooser.up')).not.toBeNull();
    chooser.click(chooser.trigger);
    expect(chooser.options().map((row) => row.firstElementChild?.textContent)).toEqual(['Gemini']);
    document.body.click();
    chooser.fixture.detectChanges();
    expect(chooser.level()).toBeUndefined();
    chooser.input('allowedAgents', ['grok']);
    expect(chooser.trigger.disabled).toBe(true);
  });

  it('keeps remembered choices in an open list and returns to models when discovery removes one', () => {
    const chooser = create();
    chooser.input('choice', { agent: 'codex', model: null, effort: null });
    chooser.input('remembered', choosing(FIRST_NEW_THREADS, { agent: 'claude', model: OPUS.id, effort: 'max' }));
    chooser.click(chooser.trigger);
    chooser.click(chooser.option('Claude'));
    expect(chooser.option('Opus 5.5').getAttribute('aria-selected')).toBe('true');
    chooser.click(chooser.option('Opus 5.5'));
    expect(chooser.option('Max').getAttribute('aria-selected')).toBe('true');
    chooser.input('models', { ...CATALOG, claude: [SONNET] });
    expect(chooser.level()).toBe('Model');
    expect(chooser.option('Sonnet 5.5')).toBeDefined();
    expect(chooser.emitted).toEqual([]);
  });
});
