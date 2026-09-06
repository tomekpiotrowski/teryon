import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { AbstractControl, FormControl, FormGroup, ReactiveFormsModule, ValidationErrors, Validators } from '@angular/forms';
import { ActivatedRoute, Router } from '@angular/router';
import { map } from 'rxjs';
import { IonContent } from '@ionic/angular/ion-content';
import { ErnoAuthService, ErnoAlertsService } from 'erno-angular';
import { ThemeService } from '../../theme.service';

function passwordsMatch(group: AbstractControl): ValidationErrors | null {
  const pw = group.get('password')?.value;
  const confirm = group.get('confirm')?.value;
  return pw && confirm && pw !== confirm ? { mismatch: true } : null;
}

@Component({
  selector: 'app-reset-password',
  templateUrl: './reset-password.component.html',
  imports: [ReactiveFormsModule, IonContent],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ResetPasswordComponent implements OnInit {
  readonly form = new FormGroup(
    {
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
  readonly ready = computed(() => this.formValid() && this.token() !== '');

  // Same reason: the mismatch notice has to react to typing, not to a render.
  readonly mismatch = toSignal(
    this.form.valueChanges.pipe(
      map(() => !!this.form.errors?.['mismatch'] && !!this.form.get('confirm')?.dirty),
    ),
    { initialValue: false },
  );
  readonly loading = signal(false);
  readonly theme = inject(ThemeService);
  private readonly token = signal('');
  private readonly formValid = toSignal(
    this.form.statusChanges.pipe(map((status) => status === 'VALID')),
    { initialValue: false },
  );

  private readonly auth = inject(ErnoAuthService);
  private readonly alerts = inject(ErnoAlertsService);
  private readonly router = inject(Router);
  private readonly route = inject(ActivatedRoute);

  ngOnInit(): void {
    this.token.set(this.route.snapshot.queryParamMap.get('token') ?? '');
    if (!this.token()) {
      this.error.set('Invalid or missing reset token.');
    }
  }

  submit(): void {
    if (!this.ready() || this.loading()) return;
    this.loading.set(true);
    this.error.set('');
    this.auth.confirmPasswordReset(this.token(), this.form.value.password!).subscribe({
      next: () => {
        // `confirmPasswordReset` returns a session, so the user is already
        // signed in — sending them to /login would only bounce back.
        this.alerts.success('Password updated.');
        this.router.navigate(['/']);
      },
      error: (e) => {
        this.error.set(e?.error?.message ?? 'Reset failed. The link may have expired.');
        this.loading.set(false);
      },
    });
  }
}
