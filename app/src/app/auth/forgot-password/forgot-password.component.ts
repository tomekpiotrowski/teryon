import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { FormControl, FormGroup, ReactiveFormsModule, Validators } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { map } from 'rxjs';
import { IonContent } from '@ionic/angular/ion-content';
import { ErnoAuthService } from 'erno-angular';
import { ThemeService } from '../../theme.service';

@Component({
  selector: 'app-forgot-password',
  templateUrl: './forgot-password.component.html',
  imports: [ReactiveFormsModule, RouterLink, IonContent],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ForgotPasswordComponent {
  readonly form = new FormGroup({
    email: new FormControl('', [Validators.required, Validators.email]),
  });
  readonly loading = signal(false);

  // Whether the form would actually submit.
  //
  // The design drops the submit button for a word that brightens to the accent
  // colour once the form is complete, so readiness has to be something the
  // template can bind to — not a `form.valid` read that only refreshes when
  // something else happens to trigger change detection.
  readonly ready = toSignal(this.form.statusChanges.pipe(map((status) => status === 'VALID')), {
    initialValue: false,
  });
  readonly done = signal(false);
  readonly theme = inject(ThemeService);

  private readonly auth = inject(ErnoAuthService);

  submit(): void {
    if (this.form.invalid || this.loading()) return;
    this.loading.set(true);
    this.auth.requestPasswordReset(this.form.value.email!).subscribe({
      next: () => { this.done.set(true); },
      error: () => { this.done.set(true); },
    });
  }
}
