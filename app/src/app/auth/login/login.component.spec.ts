import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { ErnoAuthService } from 'erno-angular';
import { LoginComponent } from './login.component';

// The design has no submit button — the only cue that the form will go through
// is the "Sign in ↵" word brightening — so `ready` is the whole affordance and
// is worth covering input by input.
describe('LoginComponent', () => {
  let component: LoginComponent;
  let fixture: ComponentFixture<LoginComponent>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [LoginComponent],
      providers: [provideRouter([])],
    }).compileComponents();

    fixture = TestBed.createComponent(LoginComponent);
    component = fixture.componentInstance;
    fixture.detectChanges();
  });

  function fill(email: string, password: string): void {
    component.form.setValue({ email, password });
  }

  it('is not ready before anything is typed', () => {
    expect(component.ready()).toBe(false);
  });

  it.each([
    ['both fields empty', '', ''],
    ['no email', '', 'hunter22'],
    ['no password', 'ada@example.com', ''],
    ['email missing an @', 'ada.example.com', 'hunter22'],
    ['email missing a domain', 'ada@', 'hunter22'],
  ])('is not ready with %s', (_case, email, password) => {
    fill(email, password);
    expect(component.ready()).toBe(false);
  });

  it('is ready once the email parses and a password is present', () => {
    fill('ada@example.com', 'x');
    expect(component.ready()).toBe(true);
  });

  it('drops back to not ready when a field is cleared again', () => {
    fill('ada@example.com', 'hunter22');
    fill('ada@example.com', '');
    expect(component.ready()).toBe(false);
  });

  it('does not submit an incomplete form', () => {
    const login = vi.spyOn(TestBed.inject(ErnoAuthService), 'login');

    fill('ada.example.com', '');
    component.submit();

    expect(login).not.toHaveBeenCalled();
    expect(component.loading()).toBe(false);
  });
});
