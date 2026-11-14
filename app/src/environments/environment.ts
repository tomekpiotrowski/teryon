function ernoApiUrl(): string {
  const fromWindow = (globalThis as { __ERNO_API_URL__?: string }).__ERNO_API_URL__;
  if (fromWindow) {
    return fromWindow;
  }
  return 'http://localhost:3000';
}

export const environment = {
  production: false,
  get apiUrl(): string {
    return ernoApiUrl();
  },
  get wsUrl(): string {
    // The API serves its live connection at `/ws`; erno connects to this
    // address exactly as given.
    return `${ernoApiUrl().replace(/^http/, 'ws')}/ws`;
  },
};
