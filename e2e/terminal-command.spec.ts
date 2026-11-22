// Actual setup interface with a simulated desktop bridge. Rust tests cover the
// filesystem; check-appimage-command.py covers packaged desktop startup.
import type { Page } from '@playwright/test';
import { expect, test } from './support';

interface Notice {
  kind: string;
  message: string;
  detail: string | null;
  replacement: string | null;
}

async function desktop(page: Page, address: string, notices: Notice[], disconnected = false) {
  const fragment = new URLSearchParams(new URL(address).hash.slice(1));
  await page.addInitScript(({ notices, port, token, disconnected }) => {
    const calls: { command: string; args: unknown }[] = [];
    Object.assign(window, {
      isTauri: true,
      setupCalls: calls,
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args: { replacement?: string | null }) => {
          calls.push({ command, args });
          if (command === 'service_connection') {
            if (disconnected) throw { reason: 'Service unavailable', restartable: false };
            return { port, token };
          }
          if (command === 'terminal_command_status') return { pending: false, notices };
          if (command === 'install_terminal_command') return { pending: false, notices: [] };
          throw new Error(`Unexpected desktop command: ${command}`);
        },
      },
    });
  }, { notices, port: Number(fragment.get('port')), token: fragment.get('token'), disconnected });
  await page.goto(address);
  const minimize = page.getByRole('dialog', { name: 'Erno Devtools' }).getByRole('button', { name: '—' });
  if (await minimize.isVisible()) await minimize.click();
}

async function installations(page: Page) {
  return page.evaluate(() => (window as unknown as { setupCalls: { command: string; args: unknown }[] }).setupCalls.filter(call => call.command === 'install_terminal_command'));
}

const conflict: Notice = { kind: 'conflict', message: '/home/test/.local/bin/teryon already exists. Teryon left it unchanged.', detail: null, replacement: 'existing-file' };
const path: Notice = { kind: 'path', message: 'Add this line to ~/.bashrc, then open a new terminal.', detail: 'export PATH="$HOME/.local/bin:$PATH"', replacement: null };

test('an existing command is replaced only by choosing the explicit action', async ({ page, service }) => {
  await desktop(page, service.address, [conflict]);
  const notice = page.getByRole('complementary', { name: 'Terminal command setup' });
  await expect(notice).toContainText('left it unchanged');
  await expect(page.getByText('Add a project to get started.')).toBeVisible();
  await notice.getByRole('button', { name: 'Replace existing command' }).click();
  await expect(notice).toHaveCount(0);
  expect(await installations(page)).toEqual([{ command: 'install_terminal_command', args: { replacement: 'existing-file' } }]);
});

test('setup guidance works even if the service is unavailable', async ({ page, service }) => {
  await desktop(page, service.address, [path], true);
  const notice = page.getByRole('complementary', { name: 'Terminal command setup' });
  await expect(notice.locator('code')).toHaveText('export PATH="$HOME/.local/bin:$PATH"');
  await expect(notice.getByRole('button', { name: 'Retry installation' })).toHaveCount(0);
  await notice.getByRole('button', { name: 'Dismiss' }).click();
  await expect(notice).toHaveCount(0);
  expect(await installations(page)).toEqual([]);
});

test('dismissing setup leaves the existing command alone', async ({ page, service }) => {
  await desktop(page, service.address, [conflict, path]);
  const notice = page.getByRole('complementary', { name: 'Terminal command setup' });
  await notice.getByRole('button', { name: 'Dismiss' }).click();
  await expect(notice).toHaveCount(0);
  expect(await installations(page)).toEqual([]);
});

test('a setup record problem is not an installation failure', async ({ page, service }) => {
  await desktop(page, service.address, [{ kind: 'diagnostic', message: 'The terminal command setup finished, but its record could not be saved: Could not create the diagnostic folder at /home/test/logs: Permission denied', detail: null, replacement: null }]);
  const notice = page.getByRole('complementary', { name: 'Terminal command setup' });
  await expect(notice).toContainText('its record could not be saved');
  await expect(notice.getByRole('button', { name: 'Retry installation' })).toHaveCount(0);
  await expect(page.getByText('Add a project to get started.')).toBeVisible();
  expect(await installations(page)).toEqual([]);
});

test('successful automatic setup needs no notice', async ({ page, service }) => {
  await desktop(page, service.address, []);
  await expect(page.getByText('Add a project to get started.')).toBeVisible();
  await expect(page.getByRole('complementary', { name: 'Terminal command setup' })).toHaveCount(0);
});

test('recovering a damaged newer release requires an explicit choice', async ({ page, service }) => {
  await desktop(page, service.address, [{ kind: 'repair', message: 'Open the newer release to repair it, or use this version.', detail: '/home/test/commands/2.0.0/teryon is missing', replacement: 'broken-newer-link' }]);
  const notice = page.getByRole('complementary', { name: 'Terminal command setup' });
  expect(await installations(page)).toEqual([]);
  await notice.getByRole('button', { name: 'Use this AppImage’s version' }).click();
  await expect(notice).toHaveCount(0);
  expect(await installations(page)).toEqual([{ command: 'install_terminal_command', args: { replacement: 'broken-newer-link' } }]);
});
