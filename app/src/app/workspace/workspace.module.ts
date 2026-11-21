import { CommonModule } from '@angular/common';
import { NgModule } from '@angular/core';
import { RouterModule } from '@angular/router';
import { IonContent } from '@ionic/angular/ion-content';
import { BoardComponent } from './board.component';
import { PlanComponent } from './plan.component';
import { CommandMenuComponent } from './command-menu.component';
import { NewProjectComponent } from './new-project.component';
import { QuestionsComponent } from './questions.component';
import { FilesComponent } from './files.component';
import { HistoryComponent } from './history.component';
import { MarkdownPipe } from './markdown.pipe';
import { RepositoryBarComponent } from './repository-bar.component';
import { UsageComponent } from './usage.component';
import { WorkspacePage } from './workspace.page';

/** The signed-in part of Teryon: the workspace screen with its menu, its file browser, its history, the board and the usage. */
@NgModule({
  declarations: [WorkspacePage, BoardComponent, PlanComponent, CommandMenuComponent, NewProjectComponent, FilesComponent, HistoryComponent, RepositoryBarComponent, UsageComponent, QuestionsComponent, MarkdownPipe],
  imports: [CommonModule, IonContent, RouterModule.forChild([{ path: '', component: WorkspacePage }])],
})
export class WorkspaceModule {}
