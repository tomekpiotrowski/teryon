import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { AbstractControl, FormControl, FormGroup, ReactiveFormsModule, ValidationErrors, Validators } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { map } from 'rxjs';
import { IonContent } from '@ionic/angular/ion-content';
import { ErnoAuthService } from 'erno-angular';
import { ThemeService } from '../../theme.service';

function passwordsMatch(group: AbstractControl): ValidationErrors | null {
  const pw = group.get('password')?.value;
  const confirm = group.get('confirm')?.value;
  return pw && confirm && pw !== confirm ? { mismatch: true } : null;
}

@Component({
  selector: 'app-register',
  templateUrl: './register.component.html',
  imports: [ReactiveFormsModule, RouterLink, IonContent],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RegisterComponent {
  readonly form = new FormGroup(
    {
      email: new FormControl('', [Validators.required, Validators.email]),
      password: new FormControl('', [Validators.required, Validators.minLength(8)]),
      confirm: new FormControl('', Validators.required),
    },
    { validators: passwordsMatch },
  );
  readonly error = signal('');

  // Whether the form would actually submit.
  //
  // The design drops the submit button for a word that brightens to the accent
  // colour once the form is complete, so readiness has to be something the
  // template can bind to — not a `form.valid` read that only refreshes when
  // something else happens to trigger change detection.
  readonly ready = toSignal(this.form.statusChanges.pipe(map((status) => status === 'VALID')), {
    initialValue: false,
  });

  // Same reason: the mismatch notice has to react to typing, not to a render.
  readonly mismatch = toSignal(
    this.form.valueChanges.pipe(
      map(() => !!this.form.errors?.['mismatch'] && !!this.form.get('confirm')?.dirty),
    ),
    { initialValue: false },
  );
  readonly loading = signal(false);
  readonly done = signal(false);
  readonly theme = inject(ThemeService);

  private readonly auth = inject(ErnoAuthService);

  submit(): void {
    if (this.form.invalid || this.loading()) return;
    this.loading.set(true);
    this.error.set('');
    const { email, password } = this.form.value;
    this.auth.register(email!, password!).subscribe({
      next: () => { this.done.set(true); },
      error: (e) => {
        this.error.set(e?.error?.message ?? 'Registration failed');
        this.loading.set(false);
      },
    });
  }
}
