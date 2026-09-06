import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { FormGroup, FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { map } from 'rxjs';
import { IonContent } from '@ionic/angular/ion-content';
import { ErnoAuthService } from 'erno-angular';
import { ThemeService } from '../../theme.service';

@Component({
  selector: 'app-login',
  templateUrl: './login.component.html',
  imports: [ReactiveFormsModule, RouterLink, IonContent],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LoginComponent {
  readonly form = new FormGroup({
    email: new FormControl('', [Validators.required, Validators.email]),
    password: new FormControl('', Validators.required),
  });
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
  readonly loading = signal(false);
  readonly theme = inject(ThemeService);

  private readonly auth = inject(ErnoAuthService);

  submit(): void {
    if (this.form.invalid || this.loading()) return;
    this.loading.set(true);
    this.error.set('');
    const { email, password } = this.form.value;
    // No navigate here: `AppComponent` moves the app when the auth state does,
    // so signing in from anywhere — including the devtools panel — lands the
    // same way.
    this.auth.login(email!, password!).subscribe({
      error: (e) => {
        this.error.set(e?.error?.message ?? 'Invalid email or password');
        this.loading.set(false);
      },
    });
  }
}
