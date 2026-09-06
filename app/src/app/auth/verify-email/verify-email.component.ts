import { ChangeDetectionStrategy, Component, inject, OnInit, signal } from '@angular/core';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { IonContent } from '@ionic/angular/ion-content';
import { ErnoAuthService, ErnoAlertsService } from 'erno-angular';
import { ThemeService } from '../../theme.service';

@Component({
  selector: 'app-verify-email',
  templateUrl: './verify-email.component.html',
  imports: [RouterLink, IonContent],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class VerifyEmailComponent implements OnInit {
  readonly state = signal<'loading' | 'error'>('loading');
  readonly error = signal('');
  readonly theme = inject(ThemeService);

  private readonly auth = inject(ErnoAuthService);
  private readonly alerts = inject(ErnoAlertsService);
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);

  ngOnInit(): void {
    const token = this.route.snapshot.queryParamMap.get('token') ?? '';
    if (!token) {
      this.state.set('error');
      this.error.set('Invalid or missing verification token.');
      return;
    }
    this.auth.verifyEmail(token).subscribe({
      next: () => {
        this.alerts.success('Email verified!');
        this.router.navigate(['/']);
      },
      error: (e) => {
        this.state.set('error');
        this.error.set(e?.error?.message ?? 'Verification failed. The link may have expired.');
      },
    });
  }
}
