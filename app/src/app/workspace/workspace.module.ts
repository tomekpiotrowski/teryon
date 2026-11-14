import { CommonModule } from '@angular/common';
import { NgModule } from '@angular/core';
import { RouterModule } from '@angular/router';
import { IonContent } from '@ionic/angular/ion-content';
import { WorkspacePage } from './workspace.page';

/** The signed-in part of Teryon: the workspace screen. */
@NgModule({
  declarations: [WorkspacePage],
  imports: [CommonModule, IonContent, RouterModule.forChild([{ path: '', component: WorkspacePage }])],
})
export class WorkspaceModule {}
