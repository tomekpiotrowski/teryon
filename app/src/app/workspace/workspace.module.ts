import { CommonModule } from '@angular/common';
import { NgModule } from '@angular/core';
import { RouterModule } from '@angular/router';
import { IonContent } from '@ionic/angular/ion-content';
import { FilesComponent } from './files.component';
import { HistoryComponent } from './history.component';
import { MarkdownPipe } from './markdown.pipe';
import { RepositoryBarComponent } from './repository-bar.component';
import { WorkspacePage } from './workspace.page';

/** The signed-in part of Teryon: the workspace screen with its file browser and its history. */
@NgModule({
  declarations: [WorkspacePage, FilesComponent, HistoryComponent, RepositoryBarComponent, MarkdownPipe],
  imports: [CommonModule, IonContent, RouterModule.forChild([{ path: '', component: WorkspacePage }])],
})
export class WorkspaceModule {}
