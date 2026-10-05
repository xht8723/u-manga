//! Job presentation metadata. Native event batches reuse an invalidated-per-book cache.
use crate::{library, types::Job};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLocation {
    pub book_id: String,
    pub book_title: String,
    pub chapter_id: String,
    pub chapter_title: String,
    pub page_number: usize,
    pub chapter_pages: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueTask {
    #[serde(flatten)]
    pub job: crate::job_view::JobView,
    pub location: Option<TaskLocation>,
    pub sequence: u64,
    pub stopping: bool,
    pub timer_running: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    pub jobs: Vec<QueueTask>,
    pub held: bool,
    pub warnings: Vec<crate::pipeline::JobFailure>,
}
pub fn submission(jobs: Vec<Job>, held: bool) -> Submission {
    let warnings = jobs
        .iter()
        .filter_map(|job| {
            job.error
                .as_ref()
                .map(|message| crate::pipeline::JobFailure {
                    id: job.id.clone(),
                    message: message.clone(),
                })
        })
        .collect();
    Submission {
        jobs: describe(jobs),
        held,
        warnings,
    }
}

#[derive(Default)]
pub struct LocationCache {
    books: HashMap<String, HashMap<String, TaskLocation>>,
    problems: HashMap<String, (std::time::Instant, String)>,
}
impl LocationCache {
    pub fn clear(&mut self) {
        self.books.clear();
        self.problems.clear();
    }
    pub fn invalidate(&mut self, path: &str) {
        self.books.remove(path);
        self.problems.remove(path);
    }
    pub fn errors(&self) -> Vec<String> {
        self.problems
            .values()
            .take(100)
            .map(|(_, e)| e.clone())
            .collect()
    }
    pub fn describe(&mut self, jobs: Vec<crate::pipeline::RuntimeJob>) -> Vec<QueueTask> {
        self.describe_views(
            jobs.into_iter()
                .map(|r| crate::job_view::RuntimeView {
                    job: crate::job_view::JobView::from_job(&r.job),
                    sequence: r.sequence,
                    stopping: r.stopping,
                    timer_running: r.timer_running,
                })
                .collect(),
        )
    }
    pub fn describe_views(&mut self, jobs: Vec<crate::job_view::RuntimeView>) -> Vec<QueueTask> {
        jobs.into_iter()
            .map(|runtime| {
                let job = runtime.job;
                if !self.books.contains_key(&job.project)
                    && self
                        .problems
                        .get(&job.project)
                        .is_none_or(|(at, _)| at.elapsed() >= std::time::Duration::from_secs(1))
                {
                    match locations(&job.project) {
                        Ok(pages) => {
                            self.books.insert(job.project.clone(), pages);
                            self.problems.remove(&job.project);
                        }
                        Err(e) => {
                            self.problems.insert(
                                job.project.clone(),
                                (
                                    std::time::Instant::now(),
                                    format!("Job labels unavailable; refresh Jobs to retry. {e}"),
                                ),
                            );
                        }
                    }
                }
                QueueTask {
                    location: self
                        .books
                        .get(&job.project)
                        .and_then(|pages| pages.get(&job.page_id))
                        .cloned(),
                    job,
                    sequence: runtime.sequence,
                    stopping: runtime.stopping,
                    timer_running: runtime.timer_running,
                }
            })
            .collect()
    }
}
fn locations(path: &str) -> anyhow::Result<HashMap<String, TaskLocation>> {
    let mut pages = HashMap::new();
    {
        let book = library::open(Path::new(path))?;
        for chapter in &book.chapters {
            for (index, id) in chapter.page_ids.iter().enumerate() {
                pages.insert(
                    id.clone(),
                    TaskLocation {
                        book_id: book.id.clone(),
                        book_title: book.metadata.title.clone(),
                        chapter_id: chapter.id.clone(),
                        chapter_title: chapter.title.clone(),
                        page_number: index + 1,
                        chapter_pages: chapter.page_ids.len(),
                    },
                );
            }
        }
    }
    Ok(pages)
}

pub fn describe(jobs: Vec<Job>) -> Vec<QueueTask> {
    let mut books: HashMap<String, HashMap<String, TaskLocation>> = HashMap::new();
    jobs.into_iter()
        .map(|job| {
            let pages = books
                .entry(job.project.clone())
                .or_insert_with(|| locations(&job.project).unwrap_or_default());
            QueueTask {
                location: pages.get(&job.page_id).cloned(),
                job: crate::job_view::JobView::from_job(&job),
                sequence: 0,
                stopping: false,
                timer_running: false,
            }
        })
        .collect()
}
