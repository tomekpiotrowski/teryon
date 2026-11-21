//! Questions a thread's agent asks the person, and what the agent is told about asking.
//!
//! Teryon starts agents without a screen to talk to, so on their own they
//! guess every unclear choice. Guessing a design choice wrong costs a whole
//! turn of work, while a question with a recommended answer costs the person
//! one keypress. The agent asks through `teryon thread ask`, the same way it
//! names its worktree and its thread, so all three agent programs can ask
//! without Teryon knowing anything about their own tools.
//!
//! The agent writes the questions as JSON. They are read here, in the
//! service, so every refusal lands in the service's log. A question takes one
//! answer or several, which [`Choosing`] keeps apart, so a question can't hold
//! a list of recommendations when it takes one answer.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

/// The fewest and most questions one call may ask.
const QUESTIONS: (usize, usize) = (1, 4);
/// The fewest and most answers a question may offer.
const ANSWERS: (usize, usize) = (2, 4);
/// The longest a question may be, in characters.
const QUESTION_LENGTH: usize = 200;
/// The longest an answer's label may be, in characters.
const LABEL_LENGTH: usize = 60;
/// The longest an answer's description may be, in characters.
const DESCRIPTION_LENGTH: usize = 200;

/// What the agent of every turn of a thread's own agent is told about asking.
///
/// Models run without a person are trained to keep going on their own
/// guesses, so this says plainly that someone reads the thread and answers.
/// It also says when not to ask, so the agent doesn't ask about every step.
pub const ASKING: &str = concat!(
    "A person reads this thread and answers questions. ",
    "When a choice about design or requirements is unclear and guessing wrong would waste real work, ask them instead of guessing. ",
    "Run `teryon thread ask` with a JSON object on standard input, then end your turn without doing more work; ",
    "their answers arrive as their next message. ",
    "Put every question you have into one call, at most four, each with two to four answers, the one you recommend first. ",
    "When more than one answer can apply, mark the question `\"several\": true` and recommend a list. ",
    "Answers are numbered from 0. `description` and `recommended` are optional. ",
    "Don't ask what you can find out by reading the project, and don't ask for permission to run commands. Example:\n\n",
    "teryon thread ask <<'EOF'\n",
    "{\"questions\": [\n",
    "  {\"question\": \"Where should the setting be stored?\",\n",
    "   \"answers\": [{\"label\": \"Config file\", \"description\": \"Survives reinstalling\"}, {\"label\": \"Database\"}],\n",
    "   \"recommended\": 0},\n",
    "  {\"question\": \"Which screens need it?\",\n",
    "   \"answers\": [{\"label\": \"Settings\"}, {\"label\": \"Thread list\"}, {\"label\": \"Code explorer\"}],\n",
    "   \"several\": true, \"recommended\": [0, 1]}\n",
    "]}\n",
    "EOF"
);

/// The brief for a turn of the thread's own agent: what it is told already, then [`ASKING`].
pub fn with_asking(brief: Option<String>) -> String {
    match brief {
        Some(brief) => format!("{brief}\n\n{ASKING}"),
        None => ASKING.to_string(),
    }
}

/// What the agent is shown after a refusal, so it can try again in the same turn.
pub const SHAPE: &str = concat!(
    "Write the questions like this:\n",
    "{\"questions\": [\n",
    "  {\"question\": \"Where should the setting be stored?\",\n",
    "   \"answers\": [{\"label\": \"Config file\", \"description\": \"Survives reinstalling\"}, {\"label\": \"Database\"}],\n",
    "   \"recommended\": 0},\n",
    "  {\"question\": \"Which screens need it?\",\n",
    "   \"answers\": [{\"label\": \"Settings\"}, {\"label\": \"Thread list\"}],\n",
    "   \"several\": true, \"recommended\": [0]}\n",
    "]}"
);

/// What the agent prints when its questions were taken.
pub const ASKED: &str = "Your questions will be shown when your turn ends. End your turn now.";

/// One call's questions, as stored in the thread and shown to the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Questions {
    pub questions: Vec<Question>,
}

/// A question and the answers on offer. The person may always answer in their own words instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Question {
    pub question: String,
    pub answers: Vec<Choice>,
    pub choosing: Choosing,
}

/// An answer on offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Choice {
    pub label: String,
    /// A line saying what the answer means. Nothing when the label says enough.
    pub description: Option<String>,
}

/// Whether a question takes one answer or several, and which the agent recommends.
///
/// The recommendations are positions in the question's answers, from 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Choosing {
    One {
        #[ts(type = "number")]
        recommended: usize,
    },
    /// Ticking none is an answer too, so the recommendations may be none.
    Several {
        #[ts(type = "Array<number>")]
        recommended: Vec<usize>,
    },
}

/// Why questions were refused. The text is for the agent, which asked.
///
/// Questions are counted from 1, as a person would; answers from 0, as `recommended` names them.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AskingError {
    #[error("Teryon couldn't read the questions: {0}.")]
    Unreadable(String),
    #[error("Ask between one and four questions at once; there are {0}.")]
    QuestionCount(usize),
    #[error("Question {question} needs two to four answers; it has {count}.")]
    AnswerCount { question: usize, count: usize },
    #[error("Question {0} is empty.")]
    EmptyQuestion(usize),
    #[error("Question {question} is {length} characters long; keep it to 200.")]
    LongQuestion { question: usize, length: usize },
    #[error("Answer {answer} of question {question} has no label.")]
    EmptyLabel { question: usize, answer: usize },
    #[error("The label of answer {answer} of question {question} is {length} characters long; keep it to 60 and put the rest in its description.")]
    LongLabel {
        question: usize,
        answer: usize,
        length: usize,
    },
    #[error("The description of answer {answer} of question {question} is {length} characters long; keep it to 200.")]
    LongDescription {
        question: usize,
        answer: usize,
        length: usize,
    },
    #[error("Question {question} offers \"{label}\" twice.")]
    RepeatedLabel { question: usize, label: String },
    #[error("Question {0} takes one answer, so `recommended` is one number, such as 0. Add `\"several\": true` if more than one answer can apply.")]
    ListForOne(usize),
    #[error("Question {0} takes several answers, so `recommended` is a list of numbers, such as [0, 2].")]
    NumberForSeveral(usize),
    #[error("Question {question} recommends answer {answer}, which isn't there; answers are numbered from 0.")]
    MissingRecommendation { question: usize, answer: usize },
    #[error("Question {question} recommends answer {answer} twice.")]
    RepeatedRecommendation { question: usize, answer: usize },
}

/// Questions as the agent writes them, before they are checked.
#[derive(Deserialize)]
struct Asked {
    questions: Vec<AskedQuestion>,
}

#[derive(Deserialize)]
struct AskedQuestion {
    question: String,
    answers: Vec<Choice>,
    #[serde(default)]
    several: bool,
    recommended: Option<Recommended>,
}

/// One recommendation or a list of them, whichever the agent wrote.
#[derive(Deserialize)]
#[serde(untagged)]
enum Recommended {
    One(usize),
    Several(Vec<usize>),
}

impl Questions {
    /// Reads and checks the JSON the agent wrote.
    pub fn read(text: &str) -> Result<Self, AskingError> {
        let asked: Asked = serde_json::from_str(text)
            .map_err(|problem| AskingError::Unreadable(problem.to_string()))?;
        let questions = asked
            .questions
            .into_iter()
            .enumerate()
            .map(|(index, asked)| {
                let choosing = match (asked.several, asked.recommended) {
                    (false, None) => Choosing::One { recommended: 0 },
                    (false, Some(Recommended::One(recommended))) => Choosing::One { recommended },
                    (false, Some(Recommended::Several(_))) => {
                        return Err(AskingError::ListForOne(index + 1))
                    }
                    (true, None) => Choosing::Several {
                        recommended: Vec::new(),
                    },
                    (true, Some(Recommended::Several(recommended))) => {
                        Choosing::Several { recommended }
                    }
                    (true, Some(Recommended::One(_))) => {
                        return Err(AskingError::NumberForSeveral(index + 1))
                    }
                };
                Ok(Question {
                    question: asked.question,
                    answers: asked.answers,
                    choosing,
                })
            })
            .collect::<Result<_, _>>()?;
        Self { questions }.checked()
    }

    /// The questions tidied, or why they can't be asked.
    ///
    /// Text is trimmed and runs of whitespace in labels are joined, since a
    /// label is one line in a list. Questions and descriptions keep their line
    /// breaks. Several recommendations are put in order.
    pub fn checked(self) -> Result<Self, AskingError> {
        let count = self.questions.len();
        if !(QUESTIONS.0..=QUESTIONS.1).contains(&count) {
            return Err(AskingError::QuestionCount(count));
        }
        let questions = self
            .questions
            .into_iter()
            .enumerate()
            .map(|(index, question)| question.checked(index + 1))
            .collect::<Result<_, _>>()?;
        Ok(Self { questions })
    }
}

impl Question {
    fn checked(self, number: usize) -> Result<Self, AskingError> {
        let question = self.question.trim().to_string();
        if question.is_empty() {
            return Err(AskingError::EmptyQuestion(number));
        }
        let length = question.chars().count();
        if length > QUESTION_LENGTH {
            return Err(AskingError::LongQuestion {
                question: number,
                length,
            });
        }
        let count = self.answers.len();
        if !(ANSWERS.0..=ANSWERS.1).contains(&count) {
            return Err(AskingError::AnswerCount {
                question: number,
                count,
            });
        }
        let mut answers: Vec<Choice> = Vec::with_capacity(count);
        for (answer, choice) in self.answers.into_iter().enumerate() {
            let choice = choice.checked(number, answer)?;
            if answers.iter().any(|earlier| earlier.label == choice.label) {
                return Err(AskingError::RepeatedLabel {
                    question: number,
                    label: choice.label,
                });
            }
            answers.push(choice);
        }
        let missing = |answer: usize| AskingError::MissingRecommendation {
            question: number,
            answer,
        };
        let choosing = match self.choosing {
            Choosing::One { recommended } if recommended >= count => {
                return Err(missing(recommended))
            }
            Choosing::One { recommended } => Choosing::One { recommended },
            Choosing::Several { mut recommended } => {
                recommended.sort_unstable();
                if let Some(&answer) = recommended.iter().find(|&&answer| answer >= count) {
                    return Err(missing(answer));
                }
                if let Some(pair) = recommended.windows(2).find(|pair| pair[0] == pair[1]) {
                    return Err(AskingError::RepeatedRecommendation {
                        question: number,
                        answer: pair[0],
                    });
                }
                Choosing::Several { recommended }
            }
        };
        Ok(Self {
            question,
            answers,
            choosing,
        })
    }
}

impl Choice {
    fn checked(self, question: usize, answer: usize) -> Result<Self, AskingError> {
        let label = self.label.split_whitespace().collect::<Vec<_>>().join(" ");
        if label.is_empty() {
            return Err(AskingError::EmptyLabel { question, answer });
        }
        let length = label.chars().count();
        if length > LABEL_LENGTH {
            return Err(AskingError::LongLabel {
                question,
                answer,
                length,
            });
        }
        let description = self
            .description
            .map(|description| description.trim().to_string())
            .filter(|description| !description.is_empty());
        if let Some(length) = description
            .as_ref()
            .map(|description| description.chars().count())
            .filter(|&length| length > DESCRIPTION_LENGTH)
        {
            return Err(AskingError::LongDescription {
                question,
                answer,
                length,
            });
        }
        Ok(Self { label, description })
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;
    use serde_json::{json, Value};

    use super::*;
    use AskingError::{
        AnswerCount, EmptyLabel, EmptyQuestion, ListForOne, LongDescription, LongLabel,
        LongQuestion, MissingRecommendation, NumberForSeveral, QuestionCount, RepeatedLabel,
        RepeatedRecommendation,
    };
    use Choosing::{One, Several};

    /// A question in the agent's JSON with these answers, and the rest given.
    fn asked(answers: Value, rest: Value) -> Value {
        let mut question = json!({ "question": "Where?", "answers": answers });
        if let (Some(question), Some(rest)) = (question.as_object_mut(), rest.as_object()) {
            question.extend(rest.clone());
        }
        question
    }

    fn two() -> Value {
        json!([{ "label": "Here" }, { "label": "There" }])
    }

    fn labels(count: usize) -> Value {
        Value::Array(
            (0..count)
                .map(|index| json!({ "label": format!("Answer {index}") }))
                .collect(),
        )
    }

    fn read(questions: Vec<Value>) -> Result<Questions, AskingError> {
        Questions::read(&json!({ "questions": questions }).to_string())
    }

    fn choosing(question: Value) -> Result<Choosing, AskingError> {
        read(vec![question]).map(|questions| questions.questions[0].choosing.clone())
    }

    /// How many answers the second question offers, once read, when it offers these.
    fn answer_count(answers: Value) -> Result<usize, AskingError> {
        read(vec![asked(two(), json!({})), asked(answers, json!({}))])
            .map(|questions| questions.questions[1].answers.len())
    }

    /// Whether a question with this text is taken.
    fn asking(question: String) -> Result<(), AskingError> {
        read(vec![asked(two(), json!({ "question": question }))]).map(|_| ())
    }

    /// Whether a question whose second answer has this label is taken.
    fn offering(label: String) -> Result<(), AskingError> {
        read(vec![asked(
            json!([{ "label": "Here" }, { "label": label }]),
            json!({}),
        )])
        .map(|_| ())
    }

    /// Whether a question whose first answer has this description is taken.
    fn describing(description: String) -> Result<(), AskingError> {
        let answers =
            json!([{ "label": "Here", "description": description }, { "label": "There" }]);
        read(vec![asked(answers, json!({}))]).map(|_| ())
    }

    fn choice(label: &str, description: Option<&str>) -> Choice {
        Choice {
            label: label.to_string(),
            description: description.map(str::to_string),
        }
    }

    lets_expect! {
        expect(ASKING) as the_brief {
            to name_the_command { have(contains("`teryon thread ask`")) be_true }
            to say_someone_answers { have(contains("A person reads this thread and answers")) be_true }
            to say_to_end_the_turn { have(contains("end your turn without doing more work")) be_true }
            to say_when_not_to_ask { have(contains("Don't ask what you can find out by reading the project")) be_true }
            to rule_out_approvals { have(contains("don't ask for permission to run commands")) be_true }
            to explain_several_answers { have(contains("`\"several\": true`")) be_true }
            to number_answers_from_zero { have(contains("Answers are numbered from 0")) be_true }
        }

        expect(with_asking(brief)) as the_author_brief {
            let brief = Some("Work in the worktree.".to_string());

            to come_after_the_rest { equal(format!("Work in the worktree.\n\n{ASKING}")) }
            when(brief = None) as be_alone_without_the_rest to hold { equal(ASKING.to_string()) }
        }

        // requirement: the example in the brief is itself accepted
        expect(Questions::read(&ASKING[ASKING.find("<<'EOF'\n").unwrap() + 8..ASKING.rfind("EOF").unwrap()]).is_ok()) as the_brief_example to be_accepted { be_true }
        expect(Questions::read(&SHAPE[SHAPE.find('{').unwrap()..]).is_ok()) as the_refusal_example to be_accepted { be_true }

        expect(read(vec![asked(json!([{ "label": "  In the\n  thread ", "description": "  Nothing\nnew \n" }, { "label": "Elsewhere", "description": "   " }]), json!({ "question": "  Where should\nanswers go?  " }))])) as a_tidied_question {
            to trim_and_join_labels_keep_line_breaks_and_drop_blank_descriptions {
                equal(Ok(Questions { questions: vec![Question {
                    question: "Where should\nanswers go?".to_string(),
                    answers: vec![choice("In the thread", Some("Nothing\nnew")), choice("Elsewhere", None)],
                    choosing: One { recommended: 0 },
                }] }))
            }
        }

        expect(choosing(question)) as what_a_question_takes {
            when(question = asked(two(), json!({}))) as recommend_the_first_by_default to hold { equal(Ok(One { recommended: 0 })) }
            when(question = asked(two(), json!({ "recommended": 1 }))) as take_a_recommendation to hold { equal(Ok(One { recommended: 1 })) }
            when(question = asked(two(), json!({ "several": false, "recommended": 1 }))) as take_one_answer_when_said to hold { equal(Ok(One { recommended: 1 })) }
            when(question = asked(two(), json!({ "recommended": [1] }))) as refuse_a_list_for_one to hold { equal(Err(ListForOne(1))) }
            when(question = asked(two(), json!({ "recommended": [] }))) as refuse_an_empty_list_for_one to hold { equal(Err(ListForOne(1))) }
            when(question = asked(two(), json!({ "recommended": 2 }))) as refuse_a_missing_answer to hold { equal(Err(MissingRecommendation { question: 1, answer: 2 })) }
            when(question = asked(two(), json!({ "recommended": -1 }))) as refuse_a_negative_number to hold { have(is_err()) be_true }
            when(question = asked(two(), json!({ "several": true }))) as recommend_nothing_by_default_for_several to hold { equal(Ok(Several { recommended: vec![] })) }
            when(question = asked(two(), json!({ "several": true, "recommended": [] }))) as take_no_recommendations to hold { equal(Ok(Several { recommended: vec![] })) }
            when(question = asked(labels(3), json!({ "several": true, "recommended": [2, 0] }))) as put_recommendations_in_order to hold { equal(Ok(Several { recommended: vec![0, 2] })) }
            when(question = asked(two(), json!({ "several": true, "recommended": 0 }))) as refuse_a_number_for_several to hold { equal(Err(NumberForSeveral(1))) }
            when(question = asked(two(), json!({ "several": true, "recommended": [0, 2] }))) as refuse_a_missing_answer_among_several to hold { equal(Err(MissingRecommendation { question: 1, answer: 2 })) }
            when(question = asked(two(), json!({ "several": true, "recommended": [1, 0, 1] }))) as refuse_a_repeated_recommendation to hold { equal(Err(RepeatedRecommendation { question: 1, answer: 1 })) }
            when(question = asked(two(), json!({ "several": "yes" }))) as refuse_several_that_is_not_true_or_false to hold { have(is_err()) be_true }
        }

        expect(read(questions).map(|questions| questions.questions.len())) as how_many_questions {
            when(questions = vec![]) as refuse_none to hold { equal(Err(QuestionCount(0))) }
            when(questions = vec![asked(two(), json!({}))]) as take_one to hold { equal(Ok(1)) }
            when(questions = vec![asked(two(), json!({})); 4]) as take_four to hold { equal(Ok(4)) }
            when(questions = vec![asked(two(), json!({})); 5]) as refuse_five to hold { equal(Err(QuestionCount(5))) }
        }

        expect(answer_count(answers)) as how_many_answers {
            let answers = labels(2);

            when(answers = labels(1)) as refuse_one to hold { equal(Err(AnswerCount { question: 2, count: 1 })) }
            when(answers = labels(0)) as refuse_none to hold { equal(Err(AnswerCount { question: 2, count: 0 })) }
            when(answers = labels(2)) as take_two to hold { equal(Ok(2)) }
            when(answers = labels(4)) as take_four to hold { equal(Ok(4)) }
            when(answers = labels(5)) as refuse_five to hold { equal(Err(AnswerCount { question: 2, count: 5 })) }
        }

        expect(asking(question)) as the_question_text {
            let question = "Where?".to_string();

            when(question = "é".repeat(200)) as take_200_characters to hold { equal(Ok(())) }
            when(question = format!("  {}  ", "é".repeat(200))) as count_after_trimming to hold { equal(Ok(())) }
            when(question = "é".repeat(201)) as refuse_201_characters to hold { equal(Err(LongQuestion { question: 1, length: 201 })) }
            when(question = " \n ".to_string()) as refuse_blank to hold { equal(Err(EmptyQuestion(1))) }
        }

        expect(offering(label)) as an_answer_label {
            let label = "There".to_string();

            when(label = "é".repeat(60)) as take_60_characters to hold { equal(Ok(())) }
            when(label = "é".repeat(61)) as refuse_61_characters to hold { equal(Err(LongLabel { question: 1, answer: 1, length: 61 })) }
            when(label = format!("{}   {}", "é".repeat(29), "é".repeat(30))) as count_after_joining to hold { equal(Ok(())) }
            when(label = "  ".to_string()) as refuse_blank to hold { equal(Err(EmptyLabel { question: 1, answer: 1 })) }
            when(label = " here ".to_string()) as take_a_label_that_differs_in_case to hold { equal(Ok(())) }
            when(label = " Here ".to_string()) as refuse_a_repeat to hold { equal(Err(RepeatedLabel { question: 1, label: "Here".to_string() })) }
        }

        expect(describing(description)) as an_answer_description {
            let description = String::new();

            when(description = "é".repeat(200)) as take_200_characters to hold { equal(Ok(())) }
            when(description = "é".repeat(201)) as refuse_201_characters to hold { equal(Err(LongDescription { question: 1, answer: 0, length: 201 })) }
        }

        expect(Questions::read(text).map(|_| ())) as the_json {
            when(text = "not json") as refuse_text to hold { have(is_err()) be_true }
            when(text = "") as refuse_nothing to hold { have(is_err()) be_true }
            when(text = r#"{"question": "Where?"}"#) as refuse_a_lone_question to hold { have(is_err()) be_true }
            when(text = r#"{"questions": [{"question": "Where?", "answers": ["Here", "There"]}]}"#) as refuse_answers_without_labels to hold { have(is_err()) be_true }
            when(text = r#"{"questions": [{"question": "Where?", "answers": [{"label": "Here"}, {"label": "There"}], "header": "Place"}]}"#) as ignore_other_fields to hold { equal(Ok(())) }
        }

        expect(Questions::read("not json").unwrap_err().to_string().starts_with("Teryon couldn't read the questions: ")) as an_unreadable_refusal to say_so { be_true }

        expect(serde_json::to_value(Several { recommended: vec![0, 2] }).unwrap()) as several_in_json to name_its_kind { equal(json!({ "kind": "several", "recommended": [0, 2] })) }
        expect(serde_json::to_value(One { recommended: 1 }).unwrap()) as one_in_json to name_its_kind { equal(json!({ "kind": "one", "recommended": 1 })) }
    }
}
