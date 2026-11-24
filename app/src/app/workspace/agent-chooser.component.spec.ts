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
  let automaticChosen = 0;
  fixture.componentInstance.automaticChosen.subscribe(() => automaticChosen++);
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
  const labels = () => options().map((row) => row.firstElementChild?.textContent);
  return { fixture, input, host, emitted, automaticChosen: () => automaticChosen, trigger, options, option, level, labels, click, key };
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

  describe('Automatic', () => {
    it('is not offered unless allowed, and the button never reads Auto', () => {
      const chooser = create();
      chooser.input('automatic', true);
      expect(chooser.trigger.textContent?.trim()).toBe('Claude · Opus 5.5 · Medium');
      chooser.click(chooser.trigger);
      expect(chooser.labels()).toEqual(['Claude', 'Codex', 'Gemini']);
      expect(chooser.option('Claude').getAttribute('aria-selected')).toBe('true');
    });

    it('is offered first and selected while on, with the first stage\'s pick behind Auto', async () => {
      const chooser = create();
      chooser.input('allowAutomatic', true);
      chooser.input('automatic', true);
      expect(chooser.trigger.textContent?.trim()).toBe('Auto · Claude · Opus 5.5 · Medium');
      expect(chooser.trigger.title).toBe('Auto · Claude · Opus 5.5 · Medium');
      chooser.click(chooser.trigger);
      await chooser.fixture.whenStable();
      expect(chooser.labels()).toEqual(['Automatic', 'Claude', 'Codex', 'Gemini']);
      expect(chooser.options().filter((row) => row.getAttribute('aria-selected') === 'true')).toEqual([chooser.option('Automatic')]);
      expect(chooser.option('Automatic').classList).toContain('agent-separated');
      expect(document.activeElement).toBe(chooser.option('Automatic'));
    });

    it('is offered unselected while a manual pick is on', () => {
      const chooser = create();
      chooser.input('allowAutomatic', true);
      expect(chooser.trigger.textContent?.trim()).toBe('Claude · Opus 5.5 · Medium');
      chooser.click(chooser.trigger);
      expect(chooser.option('Automatic').getAttribute('aria-selected')).toBe('false');
      expect(chooser.option('Claude').getAttribute('aria-selected')).toBe('true');
    });

    it.each<{ reason: string | null; label: string; title: string }>([
      { reason: 'All agents are at their limits.', label: 'Auto · Unavailable', title: 'All agents are at their limits.' },
      { reason: null, label: 'Auto', title: 'Auto' },
    ])('reads $label without a pick, with the reason on hover', ({ reason, label, title }) => {
      const chooser = create();
      chooser.input('allowAutomatic', true);
      chooser.input('automatic', true);
      chooser.input('choice', null);
      chooser.input('unavailableReason', reason);
      expect(chooser.trigger.textContent?.trim()).toBe(label);
      expect(chooser.trigger.title).toBe(title);
      expect(chooser.trigger.disabled).toBe(false);
    });

    it('reports Automatic on click and returns focus to the button', () => {
      const chooser = create();
      chooser.input('allowAutomatic', true);
      chooser.click(chooser.trigger);
      chooser.click(chooser.option('Automatic'));
      expect(chooser.automaticChosen()).toBe(1);
      expect(chooser.emitted).toEqual([]);
      expect(chooser.level()).toBeUndefined();
      expect(document.activeElement).toBe(chooser.trigger);
    });

    it('is chosen with the keyboard, and ignores the right arrow', async () => {
      const chooser = create();
      chooser.input('allowAutomatic', true);
      chooser.input('automatic', true);
      chooser.trigger.focus();
      chooser.key('ArrowDown');
      await chooser.fixture.whenStable();
      expect(document.activeElement).toBe(chooser.option('Automatic'));
      chooser.key('ArrowRight');
      expect(chooser.level()).toBe('Agent');
      expect(chooser.automaticChosen()).toBe(0);
      (document.activeElement as HTMLButtonElement).click();
      chooser.fixture.detectChanges();
      expect(chooser.automaticChosen()).toBe(1);
    });

    it('lets a manual model be picked while on', () => {
      const chooser = create();
      chooser.input('allowAutomatic', true);
      chooser.input('automatic', true);
      chooser.click(chooser.trigger);
      chooser.click(chooser.option('Claude'));
      chooser.click(chooser.option('Sonnet 5.5'));
      chooser.click(chooser.option('High'));
      expect(chooser.emitted).toEqual([{ agent: 'claude', model: SONNET.id, effort: 'high' }]);
      expect(chooser.automaticChosen()).toBe(0);
    });
  });
});
