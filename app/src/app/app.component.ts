import { TerminalCommandModule } from './terminal-command/terminal-command.module';
import { Component } from '@angular/core';
import { IonApp } from '@ionic/angular/ion-app';
import { IonRouterOutlet } from '@ionic/angular/ion-router-outlet';
import { ErnoDevtoolsComponent } from 'erno-angular';

/**
 * The frame every screen is shown in.
 *
 * It no longer sends anyone to the sign-in screen: the workspace talks to the
 * local service, not to an account on a server.
 */
@Component({
  selector: 'app-root',
  templateUrl: 'app.component.html',
  imports: [IonApp, IonRouterOutlet, ErnoDevtoolsComponent, TerminalCommandModule],
})
export class AppComponent {}
