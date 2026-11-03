"""Tests for the fields `garmin-runs --profile` keeps from Garmin's responses."""

from garmin_runs.cli import extract_profile, recorded_max_hr

ZONES = [
    {"sport": "CYCLING", "maxHeartRateUsed": 150},
    {
        "sport": "RUNNING",
        "maxHeartRateUsed": 171,
        "restingHeartRateUsed": 67,
        "lactateThresholdHeartRateUsed": None,
        "trainingMethod": "HR_MAX",
        "zone1Floor": 86,
        "zone2Floor": 103,
        "zone3Floor": 120,
        "zone4Floor": 137,
        "zone5Floor": 154,
    },
]
USER = {
    "id": 1,
    "userData": {
        "weight": 73000.0,
        "height": 174.0,
        "vo2MaxRunning": 41.0,
        "birthDate": "1980-01-01",
        "gender": "X",
        "weatherLocation": {"latitude": 57.7, "longitude": 12.1},
    },
}
METRICS = [{"generic": {"vo2MaxPreciseValue": 41.5, "fitnessAge": 42}}]
RESTING = {"allMetrics": {"metricsMap": {"WELLNESS_RESTING_HEART_RATE": [{"value": 58.0}]}}}


def test_extracts_the_training_fields():
    profile = extract_profile(ZONES, USER, METRICS, RESTING)
    assert profile["max_hr_setting"] == 171
    assert profile["resting_hr_setting"] == 67
    assert profile["resting_hr_today"] == 58.0
    assert profile["zone_floors"] == [86, 103, 120, 137, 154]
    assert profile["weight_kg"] == 73.0
    assert profile["height_cm"] == 174.0
    assert profile["vo2max_running"] == 41.5
    assert profile["fitness_age"] == 42


def test_keeps_out_personal_details():
    saved = str(extract_profile(ZONES, USER, METRICS, RESTING))
    for private in ("1980-01-01", "57.7", "12.1", "'X'"):
        assert private not in saved


def test_failed_calls_leave_blanks():
    profile = extract_profile(None, None, None, None)
    assert profile["zone_floors"] == []
    assert profile["max_hr_setting"] is None
    assert profile["weight_kg"] is None

    # Each call can fail alone, and an empty answer is not an error.
    assert extract_profile(ZONES, None, [], {})["max_hr_setting"] == 171
    assert extract_profile([], USER, None, None)["weight_kg"] == 73.0


def test_default_zones_are_the_fallback():
    zones = [{"sport": "DEFAULT", "maxHeartRateUsed": 180}]
    assert extract_profile(zones, None, None, None)["max_hr_setting"] == 180


def test_recorded_max_hr_is_runs_only(tmp_path):
    path = tmp_path / "summary.csv"
    path.write_text(
        "activity_id,date,type,max_hr\n"
        "1,2026-01-01,running,180.0\n"
        "2,2026-01-02,cycling,195.0\n"
        "3,2026-01-03,trail,185.0\n"
        "4,2026-01-04,running,\n",
        encoding="utf-8",
    )
    assert recorded_max_hr(path) == [(185, "2026-01-03"), (180, "2026-01-01")]
