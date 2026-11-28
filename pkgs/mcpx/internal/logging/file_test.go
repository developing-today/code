package logging_test

import (
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
)

func jsonlFiles(t *testing.T, dir string) []string {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(dir, "*.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	return paths
}

func countLines(t *testing.T, path string) int {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return len(strings.Split(strings.TrimRight(string(b), "\n"), "\n"))
}

func at(day time.Time, n int) logging.Record {
	return logging.Record{Time: day.Add(time.Duration(n) * time.Second),
		Level: slog.LevelInfo, Msg: "record"}
}

func TestFileSinkRotatesOnLineCountAsWellAsOnSize(t *testing.T) {
	dir := t.TempDir()
	sink, err := logging.NewFileSink(logging.FileOptions{Dir: dir, MaxLines: 3, MaxAge: -1})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = sink.Close() }()

	day := time.Now().Truncate(time.Hour)
	for i := 0; i < 5; i++ {
		sink.Write(at(day, i), nil)
	}
	files := jsonlFiles(t, dir)
	if len(files) != 2 {
		t.Fatalf("five records over a three-line limit left %d files, want 2", len(files))
	}
	if n := countLines(t, sink.Path()); n != 2 {
		t.Errorf("the live file holds %d records, want the 2 written after rotation", n)
	}
}

func TestFileSinkRotatesOnceTheOldestRecordExceedsMaxAge(t *testing.T) {
	dir := t.TempDir()
	sink, err := logging.NewFileSink(logging.FileOptions{Dir: dir, MaxAge: time.Hour, MaxLines: -1})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = sink.Close() }()

	// Every record has to fall on the same local calendar day, or the daily
	// trigger rotates the file and the age limit is not what is being
	// measured. time.Truncate works on absolute time, so it yields UTC
	// midnight -- which is the previous local day for anyone west of
	// Greenwich, and made this test fail depending on the hour it ran.
	now := time.Now()
	day := time.Date(now.Year(), now.Month(), now.Day(), 2, 0, 0, 0, now.Location())
	sink.Write(logging.Record{Time: day, Level: slog.LevelInfo, Msg: "first"}, nil)
	sink.Write(logging.Record{Time: day.Add(30 * time.Minute), Level: slog.LevelInfo, Msg: "still young"}, nil)
	if got := len(jsonlFiles(t, dir)); got != 1 {
		t.Fatalf("rotated after half the age limit: %d files", got)
	}
	sink.Write(logging.Record{Time: day.Add(2 * time.Hour), Level: slog.LevelInfo, Msg: "old enough"}, nil)
	if got := len(jsonlFiles(t, dir)); got != 2 {
		t.Fatalf("did not rotate past the age limit: %d files", got)
	}
	if n := countLines(t, sink.Path()); n != 1 {
		t.Errorf("the live file holds %d records, want only the one written after rotation", n)
	}
}

func TestTwoRotationsInOneSecondKeepBothFiles(t *testing.T) {
	dir := t.TempDir()
	sink, err := logging.NewFileSink(logging.FileOptions{Dir: dir, MaxLines: 1, MaxAge: -1})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = sink.Close() }()

	day := time.Now().Truncate(time.Hour)
	for i := 0; i < 3; i++ {
		sink.Write(logging.Record{Time: day, Level: slog.LevelInfo, Msg: "same second"}, nil)
	}
	if got := len(jsonlFiles(t, dir)); got != 3 {
		t.Fatalf("three records at a one-line limit left %d files; a rotation overwrote another", got)
	}
}

func TestReopeningAFileCountsWhatIsAlreadyInIt(t *testing.T) {
	dir := t.TempDir()
	day := time.Now().Truncate(time.Hour)

	first, err := logging.NewFileSink(logging.FileOptions{Dir: dir, MaxLines: 4, MaxAge: -1})
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 3; i++ {
		first.Write(at(day, i), nil)
	}
	_ = first.Close()

	// A restart must not reset the count, or a daemon cycling often enough
	// would never rotate at all.
	second, err := logging.NewFileSink(logging.FileOptions{Dir: dir, MaxLines: 4, MaxAge: -1})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = second.Close() }()
	second.Write(at(day, 3), nil)
	second.Write(at(day, 4), nil)
	if got := len(jsonlFiles(t, dir)); got != 2 {
		t.Fatalf("restart lost the line count: %d files, want 2", got)
	}
}

func TestRotationThresholdsComeFromTheEmbeddedDefaults(t *testing.T) {
	dir := t.TempDir()
	sink, err := logging.NewFileSink(logging.FileOptions{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = sink.Close() }()
	day := time.Now().Truncate(time.Hour)
	sink.Write(at(day, 0), nil)
	if got := len(jsonlFiles(t, dir)); got != 1 {
		t.Fatalf("a single record rotated under the default thresholds: %d files", got)
	}
}

// TestFileSinkReadsTheDayInItsOwnZoneWhateverZoneARecordCarries guards the
// file name against a record's zone. Scripts stamp records with
// toISOString, which is UTC; the day used to be read in each record's own
// zone, so whenever the local and UTC dates differed every such record
// reopened the sink on a UTC-dated file and the next local record reopened
// it back. Oracle: the sink's zone is chosen so its date differs from UTC's
// right now (asserted), and all records still land in its one day file.
func TestFileSinkReadsTheDayInItsOwnZoneWhateverZoneARecordCarries(t *testing.T) {
	now := time.Now()
	offset := 14 * time.Hour
	if now.UTC().Hour() < 10 {
		offset = -12 * time.Hour
	}
	zone := time.FixedZone("far", int(offset/time.Second))
	if now.In(zone).Format("2006-01-02") == now.UTC().Format("2006-01-02") {
		t.Fatalf("premise: the sink's date must differ from UTC's at %s", now.UTC())
	}

	dir := t.TempDir()
	sink, err := logging.NewFileSink(logging.FileOptions{Dir: dir, MaxAge: -1, MaxLines: -1, Location: zone})
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = sink.Close() }()

	at := time.Now()
	for _, record := range []logging.Record{
		{Time: at.In(zone), Level: slog.LevelInfo, Msg: "sink zone"},
		{Time: at.UTC(), Level: slog.LevelInfo, Msg: "utc, as a script stamps it"},
		{Time: at.In(zone).Add(time.Second), Level: slog.LevelInfo, Msg: "sink zone again"},
	} {
		sink.Write(record, nil)
	}

	files := jsonlFiles(t, dir)
	want := "mcpx-" + at.In(zone).Format("2006-01-02") + ".jsonl"
	if len(files) != 1 || filepath.Base(files[0]) != want {
		t.Fatalf("records spread over %v, want all in %s", files, want)
	}
	if n := countLines(t, files[0]); n != 3 {
		t.Errorf("the day file holds %d records, want 3", n)
	}
}
