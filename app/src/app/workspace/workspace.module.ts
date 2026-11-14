import { CommonModule } from '@angular/common';
import { NgModule } from '@angular/core';
import { RouterModule } from '@angular/router';
import { IonContent } from '@ionic/angular/ion-content';
import { FilesComponent } from './files.component';
import { MarkdownPipe } from './markdown.pipe';
import { WorkspacePage } from './workspace.page';

/** The signed-in part of Teryon: the workspace screen and its file browser. */
@NgModule({
  declarations: [WorkspacePage, FilesComponent, MarkdownPipe],
  imports: [CommonModule, IonContent, RouterModule.forChild([{ path: '', component: WorkspacePage }])],
})
export class WorkspaceModule {}
